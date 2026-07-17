//! **forgit**, ported from a zsh plugin to a native zshrs plugin.
//!
//! The upstream forgit (github.com/wfxr/forgit) is a set of zsh/bash
//! functions that wrap `git` + `fzf`: list something with git, pick it
//! interactively with fzf (with a live diff/preview), then act on the
//! selection. This is a faithful behavioural port — same commands, same
//! aliases, same fzf UX — with the orchestration, argument handling and
//! sequencing moved into Rust, and `git`/`fzf` driven as subprocesses.
//!
//! What stays shell: the fzf `--preview` (and `enter:execute(...)`)
//! strings. fzf runs those per-item via `sh -c`, so they are irreducibly
//! shell — exactly as in the original. Everything else is Rust.
//!
//! Commands (default aliases):
//!   ga      git add selector                (forgit::add)
//!   grh     git reset HEAD / unstage        (forgit::reset::head)
//!   glo     git log viewer                  (forgit::log)
//!   gd      git diff viewer                 (forgit::diff)
//!   gcf     git checkout-file / restore     (forgit::restore)
//!   gclean  git clean selector              (forgit::clean)
//!   gss     git stash viewer                (forgit::stash::show)
//!   gcp     git cherry-pick selector        (forgit::cherry::pick)
//!   gi      gitignore generator             (forgit::ignore)
//!
//! Requires `git` and `fzf` on PATH (same runtime deps as forgit).

use std::io::Write;
use std::os::raw::c_int;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use zshrs_plugin::{declare_plugin, Args, Host};

// ============================================================
// git / process helpers
// ============================================================

/// `git rev-parse --is-inside-work-tree` — the guard every command opens
/// with (forgit::inside_work_tree).
fn inside_work_tree() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run `git <args>`, return stdout as a String (stderr inherited). Empty
/// string on failure.
fn git_capture(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Run `git <args>` inheriting stdio (output goes straight to the shell's
/// terminal), return success. Used for the "act on the selection" step.
fn git_run(args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `git config <key>`, trimmed, or None when unset/empty.
fn git_config(key: &str) -> Option<String> {
    let out = Command::new("git").args(["config", key]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

fn toplevel() -> String {
    git_capture(&["rev-parse", "--show-toplevel"]).trim().to_string()
}

/// Spawn `fzf`, feed `input` on its stdin, return the selected lines
/// (stdout). `opts_env` becomes `FZF_DEFAULT_OPTS`; `preview` becomes
/// `--preview=<preview>`. Returns None when fzf is aborted or nothing is
/// selected. fzf drives the terminal itself (stderr inherited).
fn fzf(input: &str, opts_env: &str, preview: Option<&str>) -> Option<String> {
    let mut cmd = Command::new("fzf");
    cmd.env("FZF_DEFAULT_OPTS", opts_env);
    if let Some(p) = preview {
        cmd.arg(format!("--preview={p}"));
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = cmd.spawn().ok()?;

    // Write the candidate list on a thread so a large list can't deadlock
    // against fzf reading stdin while we wait on stdout.
    let mut stdin = child.stdin.take()?;
    let owned = input.to_string();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(owned.as_bytes());
    });
    let out = child.wait_with_output().ok()?;
    let _ = writer.join();

    if !out.status.success() {
        return None; // fzf exits non-zero on abort / no match
    }
    let sel = String::from_utf8_lossy(&out.stdout);
    let sel = sel.trim_end_matches('\n');
    (!sel.is_empty()).then(|| sel.to_string())
}

// ============================================================
// resolved config (pagers + fzf base opts), mirroring
// forgit.plugin.sh's top-of-file setup.
// ============================================================

struct Ctx {
    base_opts: String,
    diff_pager: String,
    show_pager: String,
}

impl Ctx {
    fn new(host: &Host) -> Self {
        let env = |k: &str| host.getvar(k).filter(|s| !s.is_empty());
        // forgit_pager=${FORGIT_PAGER:-$(git config core.pager || echo cat)}
        let pager = env("FORGIT_PAGER")
            .or_else(|| git_config("core.pager"))
            .unwrap_or_else(|| "cat".to_string());
        let show_pager = env("FORGIT_SHOW_PAGER")
            .or_else(|| git_config("pager.show"))
            .unwrap_or_else(|| pager.clone());
        let diff_pager = env("FORGIT_DIFF_PAGER")
            .or_else(|| git_config("pager.diff"))
            .unwrap_or_else(|| pager.clone());

        let fzf_default = env("FZF_DEFAULT_OPTS").unwrap_or_default();
        let extra = env("FORGIT_FZF_DEFAULT_OPTS").unwrap_or_default();
        // The FORGIT_FZF_DEFAULT_OPTS block, verbatim from forgit.plugin.sh.
        let base_opts = format!(
            "{fzf_default}\n\
             --ansi\n\
             --height=80%\n\
             --bind=alt-k:preview-up,alt-p:preview-up\n\
             --bind=alt-j:preview-down,alt-n:preview-down\n\
             --bind=ctrl-r:toggle-all\n\
             --bind=ctrl-s:toggle-sort\n\
             --bind=?:toggle-preview\n\
             --bind=alt-w:toggle-preview-wrap\n\
             --preview-window=right:60%\n\
             +1\n\
             {extra}"
        );
        Ctx {
            base_opts,
            diff_pager,
            show_pager,
        }
    }
}

/// `git status --short` — printed after a mutating command, like forgit.
fn print_status_short() {
    let _ = git_run(&["status", "--short"]);
}

// ============================================================
// commands
// ============================================================

/// `ga` — forgit::add. Interactive `git add`. With arguments, adds them
/// directly (`git add "$@"`); otherwise fzf-picks modified/untracked
/// files (via porcelain, parsed in Rust) and stages the selection.
fn ga(host: &Host, args: &Args) -> c_int {
    if !inside_work_tree() {
        return 1;
    }
    let rest = args.rest();
    if !rest.is_empty() {
        let mut a = vec!["add"];
        a.extend(rest.iter().map(String::as_str));
        if git_run(&a) {
            let _ = git_run(&["status", "-su"]);
            return 0;
        }
        return 1;
    }

    let ctx = Ctx::new(host);
    // Robust file list from porcelain (path in col 4+, XY status in 1..3).
    // Skips staged-only entries the way forgit's color-grep does.
    let porcelain = git_capture(&["status", "--porcelain", "-u"]);
    let mut list = String::new();
    for line in porcelain.lines() {
        if line.len() < 4 {
            continue;
        }
        let (xy, path) = line.split_at(2);
        let path = path.trim_start();
        // worktree-side change present (unstaged/untracked): col 2 != ' '
        // OR untracked '??'. Matches forgit's changed/unmerged/untracked.
        let wt = xy.as_bytes().get(1).copied().unwrap_or(b' ');
        if xy == "??" || wt != b' ' || xy.starts_with('U') {
            list.push_str(path);
            list.push('\n');
        }
    }
    if list.trim().is_empty() {
        host.print("Nothing to add.\n");
        return 0;
    }
    // Preview: untracked -> diff against /dev/null, else normal diff.
    let preview = format!(
        "f={{}}; if git status -s -- \"$f\" | grep -q '^??'; then \
           git diff --color=always --no-index -- /dev/null \"$f\" | {dp} | sed '2 s/added:/untracked:/'; \
         else git diff --color=always -- \"$f\" | {dp}; fi",
        dp = ctx.diff_pager
    );
    let opts = format!("{}\n-0 -m", ctx.base_opts);
    let Some(sel) = fzf(&list, &opts, Some(&preview)) else {
        host.print("Nothing to add.\n");
        return 0;
    };
    let files: Vec<&str> = sel.lines().filter(|l| !l.is_empty()).collect();
    let mut a = vec!["add"];
    a.extend(files.iter().copied());
    let _ = git_run(&a);
    let _ = git_run(&["status", "-su"]);
    0
}

/// `grh` — forgit::reset::head. Unstage selected files.
fn grh(host: &Host, _args: &Args) -> c_int {
    if !inside_work_tree() {
        return 1;
    }
    let ctx = Ctx::new(host);
    let list = git_capture(&["diff", "--cached", "--name-only", "--relative"]);
    if list.trim().is_empty() {
        host.print("Nothing to unstage.\n");
        return 0;
    }
    let preview = format!("git diff --cached --color=always -- {{}} | {}", ctx.diff_pager);
    let opts = format!("{}\n-m -0", ctx.base_opts);
    let Some(sel) = fzf(&list, &opts, Some(&preview)) else {
        host.print("Nothing to unstage.\n");
        return 0;
    };
    for f in sel.lines().filter(|l| !l.is_empty()) {
        let _ = git_run(&["reset", "-q", "HEAD", f]);
    }
    print_status_short();
    0
}

/// `gcf` — forgit::restore. `git checkout` selected modified files.
fn gcf(host: &Host, _args: &Args) -> c_int {
    if !inside_work_tree() {
        return 1;
    }
    let ctx = Ctx::new(host);
    let top = toplevel();
    let list = git_capture(&["ls-files", "--modified", &top]);
    if list.trim().is_empty() {
        host.print("Nothing to restore.\n");
        return 0;
    }
    let preview = format!("git diff --color=always -- {{}} | {}", ctx.diff_pager);
    let opts = format!("{}\n-m -0", ctx.base_opts);
    let Some(sel) = fzf(&list, &opts, Some(&preview)) else {
        host.print("Nothing to restore.\n");
        return 0;
    };
    for f in sel.lines().filter(|l| !l.is_empty()) {
        let _ = git_run(&["checkout", f]);
    }
    print_status_short();
    0
}

/// `gclean` — forgit::clean. `git clean` selected untracked entries.
fn gclean(host: &Host, args: &Args) -> c_int {
    if !inside_work_tree() {
        return 1;
    }
    let ctx = Ctx::new(host);
    let mut ca = vec!["clean", "-xdffn"];
    ca.extend(args.rest().iter().map(String::as_str));
    let raw = git_capture(&ca);
    let mut list = String::new();
    for line in raw.lines() {
        let f = line.strip_prefix("Would remove ").unwrap_or(line);
        if !f.is_empty() {
            list.push_str(f);
            list.push('\n');
        }
    }
    if list.trim().is_empty() {
        host.print("Nothing to clean.\n");
        return 0;
    }
    let Some(sel) = fzf(&list, &format!("{}\n-m -0", ctx.base_opts), None) else {
        host.print("Nothing to clean.\n");
        return 0;
    };
    for f in sel.lines().filter(|l| !l.is_empty()) {
        let f = f.strip_suffix('/').unwrap_or(f); // dir path needs no trailing /
        let _ = git_run(&["clean", "-xdff", f]);
    }
    print_status_short();
    0
}

/// `glo` — forgit::log. Commit viewer: fzf over the graph log, preview
/// the commit, `enter` opens it in a pager.
fn glo(host: &Host, args: &Args) -> c_int {
    if !inside_work_tree() {
        return 1;
    }
    let ctx = Ctx::new(host);
    // Extra files after `--` are passed to `git show` in the preview.
    let files = args
        .rest()
        .iter()
        .position(|a| a == "--")
        .map(|i| args.rest()[i + 1..].join(" "))
        .unwrap_or_default();
    let show = format!(
        "echo {{}} | grep -Eo '[a-f0-9]+' | head -1 | \
         xargs -I% git show --color=always % -- {files} | {sp}",
        files = files,
        sp = ctx.show_pager
    );
    let opts = format!(
        "{base}\n+s +m --tiebreak=index\n\
         --bind=enter:execute({show} | LESS=-R less)\n\
         --bind=ctrl-y:execute-silent(echo {{}} | grep -Eo '[a-f0-9]+' | head -1 | tr -d '\\n' | {copy})",
        base = ctx.base_opts,
        show = show,
        copy = std::env::var("FORGIT_COPY_CMD").unwrap_or_else(|_| "pbcopy".to_string())
    );
    let mut la = vec![
        "log",
        "--graph",
        "--color=always",
        "--format=%C(auto)%h%d %s %C(black)%C(bold)%cr",
    ];
    // Positional args (revisions/paths) pass through to git log.
    la.extend(args.rest().iter().map(String::as_str));
    let list = git_capture(&la);
    let _ = fzf(&list, &opts, Some(&show));
    0
}

/// `gd` — forgit::diff. Diff viewer over changed files; optional leading
/// commit-ish argument.
fn gd(host: &Host, args: &Args) -> c_int {
    if !inside_work_tree() {
        return 1;
    }
    let ctx = Ctx::new(host);
    let rest = args.rest();
    // If arg 1 is a valid rev, treat it as the commit; the rest are paths.
    let (commit, files): (String, Vec<&str>) = match rest.first() {
        Some(first)
            if Command::new("git")
                .args(["rev-parse", first, "--"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false) =>
        {
            (first.clone(), rest[1..].iter().map(String::as_str).collect())
        }
        _ => (String::new(), rest.iter().map(String::as_str).collect()),
    };
    let repo = toplevel();
    let preview = format!(
        "echo {{}} | sed 's/.*]  //' | xargs -I% git diff --color=always {c} -- '{repo}/%' | {dp}",
        c = commit,
        repo = repo,
        dp = ctx.diff_pager
    );
    let opts = format!(
        "{base}\n+m -0\n--bind=enter:execute({preview} | LESS=-R less)",
        base = ctx.base_opts,
        preview = preview
    );
    let mut da = vec!["diff", "--name-status"];
    if !commit.is_empty() {
        da.push(&commit);
    }
    da.push("--");
    da.extend(files.iter().copied());
    let raw = git_capture(&da);
    // `X<tab/space>path` -> `[X]  path` (forgit's sed).
    let mut list = String::new();
    for line in raw.lines() {
        if let Some((st, path)) = line.split_once(|c: char| c.is_whitespace()) {
            list.push_str(&format!("[{}]  {}\n", st.trim(), path.trim_start()));
        }
    }
    let _ = fzf(&list, &opts, Some(&preview));
    0
}

/// `gss` — forgit::stash::show. Stash viewer.
fn gss(host: &Host, _args: &Args) -> c_int {
    if !inside_work_tree() {
        return 1;
    }
    let ctx = Ctx::new(host);
    let list = git_capture(&["stash", "list"]);
    if list.trim().is_empty() {
        host.print("No stashes.\n");
        return 0;
    }
    let preview = format!(
        "echo {{}} | cut -d: -f1 | xargs -I% git stash show --color=always --ext-diff % | {}",
        ctx.diff_pager
    );
    let opts = format!(
        "{base}\n+s +m -0 --tiebreak=index\n--bind=enter:execute({preview} | LESS=-R less)",
        base = ctx.base_opts,
        preview = preview
    );
    let _ = fzf(&list, &opts, Some(&preview));
    0
}

/// `gcp` — forgit::cherry::pick. Pick commits from a target branch that
/// are not on the current branch and cherry-pick them.
fn gcp(host: &Host, args: &Args) -> c_int {
    let base = git_capture(&["branch", "--show-current"]);
    let base = base.trim();
    let Some(target) = args.rest().first() else {
        host.print("Please specify target branch\n");
        return 1;
    };
    let ctx = Ctx::new(host);
    let raw = git_capture(&["cherry", base, target, "--abbrev", "-v"]);
    // `+ <sha> <subject>` -> drop the leading marker (cut -d' ' -f2-).
    let mut list = String::new();
    for line in raw.lines() {
        let rest = line.splitn(2, ' ').nth(1).unwrap_or(line);
        list.push_str(rest);
        list.push('\n');
    }
    if list.trim().is_empty() {
        host.print("Nothing to cherry-pick.\n");
        return 0;
    }
    let preview = format!(
        "echo {{}} | cut -d' ' -f1 | xargs -I% git show --color=always % | {}",
        ctx.show_pager
    );
    let Some(sel) = fzf(&list, &format!("{}\n-m -0", ctx.base_opts), Some(&preview)) else {
        return 0;
    };
    for line in sel.lines().filter(|l| !l.is_empty()) {
        if let Some(sha) = line.split(' ').next() {
            let _ = git_run(&["cherry-pick", sha]);
        }
    }
    0
}

/// `gi` — forgit::ignore. gitignore template generator. Clones the
/// template repo on first use, fzf-picks templates, prints them.
fn gi(host: &Host, args: &Args) -> c_int {
    let env = |k: &str| host.getvar(k).filter(|s| !s.is_empty());
    let repo_remote = env("FORGIT_GI_REPO_REMOTE")
        .unwrap_or_else(|| "https://github.com/dvcs/gitignore".to_string());
    let home = host.getvar("HOME").unwrap_or_default();
    let repo_local = env("FORGIT_GI_REPO_LOCAL")
        .unwrap_or_else(|| format!("{home}/.forgit/gi/repos/dvcs/gitignore"));
    let templates = env("FORGIT_GI_TEMPLATES").unwrap_or_else(|| format!("{repo_local}/templates"));

    if !PathBuf::from(&templates).is_dir() {
        host.print("[Info] Initializing gitignore repo...\n");
        if !git_run(&["clone", "--depth=1", &repo_remote, &repo_local]) {
            host.print("gi: failed to clone template repo\n");
            return 1;
        }
    }

    // Requested templates, or fzf-pick when none given.
    let mut names: Vec<String> = args.rest().iter().cloned().collect();
    if names.is_empty() {
        let mut all: Vec<String> = std::fs::read_dir(&templates)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                Some(n.strip_suffix(".gitignore").unwrap_or(&n).to_string())
            })
            .collect();
        all.sort_unstable_by_key(|s| s.to_lowercase());
        all.dedup();
        let ctx = Ctx::new(host);
        let preview = format!("cat '{templates}/'{{}}'.gitignore' 2>/dev/null || cat '{templates}/'{{}} 2>/dev/null");
        let opts = format!("{}\n-m --preview-window=right:70%", ctx.base_opts);
        let Some(sel) = fzf(&all.join("\n"), &opts, Some(&preview)) else {
            return 1;
        };
        names = sel.lines().map(|s| s.to_string()).collect();
    }
    if names.is_empty() {
        return 1;
    }
    // Emit each template with a header (forgit::ignore::get).
    let mut out = String::new();
    for item in &names {
        let cands = [
            format!("{templates}/{item}.gitignore"),
            format!("{templates}/{item}"),
        ];
        if let Some(path) = cands.iter().find(|p| PathBuf::from(p).is_file()) {
            if let Ok(body) = std::fs::read_to_string(path) {
                let header = PathBuf::from(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push_str(&format!("### {header}\n{body}\n"));
            }
        } else {
            host.print(&format!("[Warn] No gitignore template found for '{item}'.\n"));
        }
    }
    host.print(&out);
    0
}

declare_plugin! {
    name: "forgit",
    version: "0.1.0",
    builtins: {
        "ga"     => ga,
        "grh"    => grh,
        "glo"    => glo,
        "gd"     => gd,
        "gcf"    => gcf,
        "gclean" => gclean,
        "gss"    => gss,
        "gcp"    => gcp,
        "gi"     => gi,
    },
}
