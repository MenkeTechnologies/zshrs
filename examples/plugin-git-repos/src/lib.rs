//! **git-repos** — zsh-git-repo-cache ported to a native zshrs plugin.
//!
//! Scans the filesystem for every git repository, caches the list, and
//! fzf-picks one to `cd` into — with optional clean/dirty filtering. The
//! shell version does this with `sudo find / -name .git` + a sequential
//! `git status` per repo; the native version does a fast in-process walk
//! and classifies clean/dirty **in parallel across threads**, which is
//! where the speedup is.
//!
//! Commands:
//!   gitrepos [--clean|--dirty] [--regen] [--list] [--root DIR]
//!     (no args)   fzf-pick any cached repo and cd to it
//!     --clean     only repos with a clean working tree
//!     --dirty     only repos with uncommitted/untracked changes
//!     --list      print (don't fzf/cd)
//!     --regen     rescan the filesystem and rebuild the cache
//!     --root DIR  scan root for --regen (default: $ZPWR_GIT_SCAN_ROOT or $HOME)
//!
//! Cache: `$ZPWR_ALL_GIT_DIRS` (default `~/.zsh-git-repo-cache`).

use std::io::Write;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use zshrs_plugin::{declare_plugin, Args, Host};

fn cfg(host: &Host, key: &str) -> Option<String> {
    host.getvar(key).filter(|s| !s.is_empty())
}

fn home(host: &Host) -> String {
    cfg(host, "HOME")
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_default()
}

fn cache_file(host: &Host) -> String {
    cfg(host, "ZPWR_ALL_GIT_DIRS").unwrap_or_else(|| format!("{}/.zsh-git-repo-cache", home(host)))
}

fn scan_root(host: &Host) -> String {
    cfg(host, "ZPWR_GIT_SCAN_ROOT").unwrap_or_else(|| home(host))
}

// ============================================================
// filesystem walk — find every git repo under `root`
// ============================================================

/// Iteratively walk `root`, collecting the parent directory of every
/// `.git` (dir or file — submodules/worktrees use a `.git` file). Does not
/// descend into `.git` or follow symlinks (avoids loops); keeps scanning
/// siblings/children so nested repos are found, mirroring `find -prune`.
fn walk_repos(root: &str) -> Vec<String> {
    let mut repos = Vec::new();
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue; // permission denied etc. — skip silently
        };
        for entry in rd.flatten() {
            let name = entry.file_name();
            if name == ".git" {
                // repo root is `dir`; do not descend into .git
                repos.push(dir.to_string_lossy().into_owned());
                continue;
            }
            // Only descend into real subdirectories (no symlinks).
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    repos.sort();
    repos.dedup();
    repos
}

/// Rebuild the cache from a fresh scan; returns the repo list.
fn regen(host: &Host, root: &str) -> Vec<String> {
    let repos = walk_repos(root);
    let cache = cache_file(host);
    let mut out = String::with_capacity(repos.len() * 40);
    for r in &repos {
        out.push_str(r);
        out.push('\n');
    }
    let _ = std::fs::write(&cache, out);
    repos
}

/// Load the cache, regenerating if it's missing or empty. Prunes repos
/// whose directory no longer exists.
fn load_repos(host: &Host) -> Vec<String> {
    let cache = cache_file(host);
    let text = std::fs::read_to_string(&cache).unwrap_or_default();
    let mut repos: Vec<String> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .collect();
    if repos.is_empty() {
        repos = regen(host, &scan_root(host));
    }
    repos.retain(|r| Path::new(r).is_dir());
    repos
}

// ============================================================
// clean/dirty classification — parallel git status
// ============================================================

/// A repo is "clean" when there are no staged/unstaged changes to tracked
/// files (`git diff-index --quiet HEAD`) AND no untracked files
/// (`git ls-files --others --exclude-standard` is empty) — exactly the
/// zsh-git-repo-cache test.
fn is_clean(repo: &str) -> bool {
    let tracked_clean = Command::new("git")
        .args(["-C", repo, "diff-index", "--quiet", "HEAD", "--"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !tracked_clean {
        return false;
    }
    let untracked = Command::new("git")
        .args(["-C", repo, "ls-files", "--others", "--exclude-standard"])
        .stderr(Stdio::null())
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();
    untracked.iter().all(|b| b.is_ascii_whitespace())
}

/// Partition repos into clean/dirty, running `git status` across a bounded
/// pool of threads (the sequential shell version's slow part).
fn classify(repos: &[String], want_clean: bool) -> Vec<String> {
    let n_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16);
    let queue = Arc::new(Mutex::new(repos.to_vec()));
    let out = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut handles = Vec::new();
    for _ in 0..n_threads {
        let queue = Arc::clone(&queue);
        let out = Arc::clone(&out);
        handles.push(std::thread::spawn(move || loop {
            let repo = {
                let mut q = queue.lock().unwrap();
                match q.pop() {
                    Some(r) => r,
                    None => break,
                }
            };
            if is_clean(&repo) == want_clean {
                out.lock().unwrap().push(repo);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    let mut v = Arc::try_unwrap(out).unwrap().into_inner().unwrap();
    v.sort();
    v
}

// ============================================================
// fzf + cd
// ============================================================

fn fzf_pick(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    let mut child = Command::new("fzf")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let input = lines.join("\n");
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let out = child.wait_with_output().ok()?;
    let _ = writer.join();
    if !out.status.success() {
        return None;
    }
    let sel = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!sel.is_empty()).then_some(sel)
}

fn shquote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// ============================================================
// command
// ============================================================

fn gitrepos(host: &Host, args: &Args) -> c_int {
    let mut clean = false;
    let mut dirty = false;
    let mut list = false;
    let mut do_regen = false;
    let mut root: Option<String> = None;

    let rest = args.rest();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--clean" => clean = true,
            "--dirty" => dirty = true,
            "--list" | "-l" => list = true,
            "--regen" | "regen" => do_regen = true,
            "--root" => {
                i += 1;
                root = rest.get(i).cloned();
            }
            "-h" | "--help" => {
                host.print("gitrepos [--clean|--dirty] [--regen] [--list] [--root DIR]\n");
                return 0;
            }
            other => {
                host.print(&format!("gitrepos: unknown option `{other}`\n"));
                return 1;
            }
        }
        i += 1;
    }

    let mut repos = if do_regen {
        let r = root.clone().unwrap_or_else(|| scan_root(host));
        let repos = regen(host, &r);
        host.print(&format!("gitrepos: cached {} repos from {}\n", repos.len(), r));
        repos
    } else {
        load_repos(host)
    };

    if clean {
        repos = classify(&repos, true);
    } else if dirty {
        repos = classify(&repos, false);
    }

    if do_regen && !list && !clean && !dirty {
        return 0; // regen-only: cache written, nothing to pick
    }

    if list {
        for r in &repos {
            host.print(&format!("{r}\n"));
        }
        return if repos.is_empty() { 1 } else { 0 };
    }

    match fzf_pick(&repos) {
        Some(path) => host.eval(&format!("cd -- {}", shquote(&path))),
        None => 1,
    }
}

declare_plugin! {
    name: "git-repos",
    version: "0.1.0",
    builtins: {
        "gitrepos" => gitrepos,
    },
}
