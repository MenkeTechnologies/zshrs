//! p10k git status backend — native replacement for gitstatusd.
//!
//! Phase-1 implementation strategy:
//! - Repo discovery + branch/commit/action come from reading `.git` directly:
//!   `.git` may be a file containing `gitdir: <path>` (worktrees/submodules);
//!   `HEAD` is either a symbolic ref (`ref: refs/heads/<branch>`) or a detached
//!   commit hash; in-progress action state (rebase/merge/cherry-pick/bisect/
//!   revert/am) is detected from marker files/dirs in the gitdir, mirroring
//!   gitstatus's action names exactly (gitstatus/src/git.cc `RepoState`, which
//!   itself mirrors libgit2 `git_repository_state`). Branch tips resolve via
//!   loose refs then `packed-refs` in the common dir.
//! - Counts (staged/unstaged/untracked/conflicted/stashes/ahead/behind) come
//!   from ONE subprocess: `git status --porcelain=v2 --branch --show-stash`.
//!   TODO(phase-2): replace the subprocess with a native index/refs parse —
//!   gitstatusd is dead on this box and a single `git` exec is the acceptable
//!   phase-1 bridge.
//! - Tag lookup is OPTIONAL phase-1: `tag` stays empty. TODO(phase-2): port
//!   gitstatus/src/tag_db.cc (packed-refs tag scan keyed by commit).
//! - Cache: per-gitdir entry with ~2s TTL, additionally invalidated when
//!   `.git/index` or `.git/HEAD` mtime changes. The prompt is never blocked
//!   more than ~45ms: the subprocess output is read on a helper thread and
//!   awaited via `mpsc::recv_timeout`; on timeout the child is killed and the
//!   cached value (stale ok) — or None — is returned.
//! - No new crate dependencies; std only.
//!
//! p10k consumes these fields as the `VCS_STATUS_*` parameters that gitstatusd
//! used to publish (p10k:3775-3783 — `$VCS_STATUS_COMMIT`,
//! `$VCS_STATUS_LOCAL_BRANCH`, `$VCS_STATUS_REMOTE_BRANCH`,
//! `$VCS_STATUS_ACTION`, `$VCS_STATUS_NUM_STAGED`, `$VCS_STATUS_NUM_UNSTAGED`,
//! `$VCS_STATUS_NUM_CONFLICTED`, `$VCS_STATUS_NUM_UNTRACKED`,
//! `$VCS_STATUS_COMMITS_AHEAD`, `$VCS_STATUS_COMMITS_BEHIND`,
//! `$VCS_STATUS_STASHES`, `$VCS_STATUS_TAG`).

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// One snapshot of a repo's state, shaped after gitstatusd's response fields
/// (gitstatus/src/response.cc) as p10k reads them (p10k:3775-3783).
#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    pub branch: String,
    pub commit: String,
    pub action: String, // rebase/merge/etc, "" if none
    pub ahead: i64,
    pub behind: i64,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
    pub conflicted: i64,
    pub stashes: i64,
    pub tag: String,
    pub remote_branch: String,
}

/// Cache TTL. gitstatusd recomputed on every prompt but kept the repo open;
/// we amortize the subprocess instead.
const CACHE_TTL: Duration = Duration::from_secs(2);
/// Prompt latency budget for the `git status` subprocess.
const SUBPROCESS_BUDGET: Duration = Duration::from_millis(45);

struct CacheEntry {
    at: Instant,
    head_mtime: Option<SystemTime>,
    index_mtime: Option<SystemTime>,
    status: GitStatus,
}

fn cache() -> &'static Mutex<HashMap<PathBuf, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Public entry point per the p10k module contract. `dir` is any directory;
/// the repo is discovered by walking up. None when not inside a git repo, or
/// when the first-ever status of a repo exceeds the latency budget.
pub fn git_status_for(dir: &Path) -> Option<GitStatus> {
    let repo = discover_repo(dir)?;
    let head_mtime = mtime_of(&repo.git_dir.join("HEAD"));
    let index_mtime = mtime_of(&repo.git_dir.join("index"));

    // Fresh-enough cache hit: TTL not expired AND HEAD/index untouched.
    if let Ok(map) = cache().lock() {
        if let Some(e) = map.get(&repo.git_dir) {
            if e.at.elapsed() < CACHE_TTL
                && e.head_mtime == head_mtime
                && e.index_mtime == index_mtime
            {
                return Some(e.status.clone());
            }
        }
    }

    let mut status = GitStatus::default();
    read_head(&repo, &mut status);
    status.action = repo_action(&repo.git_dir); // gitstatus git.cc:43-110 RepoState

    match run_porcelain(&repo.work_dir) {
        Some(out) => {
            parse_porcelain_v2(&out, &mut status);
            if let Ok(mut map) = cache().lock() {
                // Bound the cache: one entry per repo visited; a
                // long-lived shell hopping across many repos must not
                // grow this without limit. Evict the stalest entry
                // once over the cap (cap is generous — the map is
                // per-repo, not per-directory).
                const CACHE_CAP: usize = 64;
                if map.len() >= CACHE_CAP && !map.contains_key(&repo.git_dir) {
                    if let Some(oldest) = map
                        .iter()
                        .max_by_key(|(_, e)| e.at.elapsed())
                        .map(|(k, _)| k.clone())
                    {
                        map.remove(&oldest);
                    }
                }
                map.insert(
                    repo.git_dir.clone(),
                    CacheEntry {
                        at: Instant::now(),
                        head_mtime,
                        index_mtime,
                        status: status.clone(),
                    },
                );
            }
            Some(status)
        }
        // Subprocess overran the budget: serve the stale cached value if we
        // have one (counts slightly old beats a blocked prompt), else None.
        None => {
            if let Ok(map) = cache().lock() {
                if let Some(e) = map.get(&repo.git_dir) {
                    return Some(e.status.clone());
                }
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Repo discovery
// ---------------------------------------------------------------------------

struct Repo {
    /// Directory containing HEAD, index, rebase state, ... (per-worktree).
    git_dir: PathBuf,
    /// Directory containing refs/, packed-refs (shared across worktrees).
    common_dir: PathBuf,
    /// Top-level working directory (where `git status` runs).
    work_dir: PathBuf,
}

/// Walk up from `dir` until a `.git` entry is found. A `.git` directory is
/// the gitdir itself; a `.git` file holds `gitdir: <path>` (worktree or
/// submodule checkout). The common dir is the gitdir unless a `commondir`
/// file redirects it (linked worktrees).
fn discover_repo(dir: &Path) -> Option<Repo> {
    let mut cur = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(dir)
    };
    loop {
        let dotgit = cur.join(".git");
        if let Ok(meta) = fs::symlink_metadata(&dotgit) {
            let git_dir = if meta.is_dir() {
                dotgit
            } else {
                // `.git` file: `gitdir: /abs/path` or `gitdir: ../rel/path`
                let text = fs::read_to_string(&dotgit).ok()?;
                let target = text.strip_prefix("gitdir:")?.trim();
                let p = Path::new(target);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    cur.join(p)
                }
            };
            let common_dir = match fs::read_to_string(git_dir.join("commondir")) {
                Ok(text) => {
                    let t = text.trim();
                    let p = Path::new(t);
                    if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        git_dir.join(p)
                    }
                }
                Err(_) => git_dir.clone(),
            };
            return Some(Repo {
                git_dir,
                common_dir,
                work_dir: cur,
            });
        }
        if !cur.pop() {
            return None;
        }
    }
}

fn mtime_of(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).ok().and_then(|m| m.modified().ok())
}

// ---------------------------------------------------------------------------
// HEAD / refs
// ---------------------------------------------------------------------------

/// Fill `branch` and `commit` from HEAD. Symbolic HEAD names the branch and
/// the tip commit resolves through loose refs then packed-refs; detached
/// HEAD is a bare hash with no branch (gitstatusd reports LOCAL_BRANCH=""
/// for detached HEAD; p10k then falls back to the commit, p10k:3775).
fn read_head(repo: &Repo, status: &mut GitStatus) {
    let head = match fs::read_to_string(repo.git_dir.join("HEAD")) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("p10k git: unreadable HEAD in {:?}: {e}", repo.git_dir);
            return;
        }
    };
    let head = head.trim();
    if let Some(refname) = head.strip_prefix("ref: ") {
        let refname = refname.trim();
        status.branch = refname
            .strip_prefix("refs/heads/")
            .unwrap_or(refname)
            .to_string();
        if let Some(oid) = resolve_ref(&repo.common_dir, refname) {
            status.commit = oid;
        }
    } else if head.len() >= 40 && head.bytes().all(|b| b.is_ascii_hexdigit()) {
        status.commit = head.to_string(); // detached HEAD
    }
}

/// Resolve a fully-qualified ref to its commit hash: loose ref file first,
/// then a linear scan of packed-refs (`<oid> <refname>` lines; `^<oid>`
/// peel lines and `#` comments skipped).
fn resolve_ref(common_dir: &Path, refname: &str) -> Option<String> {
    if let Ok(s) = fs::read_to_string(common_dir.join(refname)) {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    let packed = fs::read_to_string(common_dir.join("packed-refs")).ok()?;
    for line in packed.lines() {
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        if let Some((oid, name)) = line.split_once(' ') {
            if name == refname {
                return Some(oid.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// In-progress action
// ---------------------------------------------------------------------------

/// Port of gitstatus RepoState (gitstatus/src/git.cc:43-110), which maps
/// libgit2 `git_repository_state` to names that "mostly match gitaction in
/// vcs_info" (git.cc:46-47). Detection order and marker files follow libgit2
/// repository.c: rebase-merge (interactive?) > rebase-apply
/// (rebasing/applying?) > MERGE_HEAD > REVERT_HEAD > CHERRY_PICK_HEAD >
/// BISECT_LOG. A rebase/am in flight appends " <next>/<last>" progress
/// (git.cc:96-108).
fn repo_action(git_dir: &Path) -> String {
    let exists = |name: &str| git_dir.join(name).exists();
    let read1 = |name: &str| -> String {
        // git.cc:87-92 ReadFile — first whitespace-delimited token.
        fs::read_to_string(git_dir.join(name))
            .ok()
            .and_then(|s| s.split_whitespace().next().map(str::to_string))
            .unwrap_or_default()
    };

    let state: &str = if exists("rebase-merge") {
        if exists("rebase-merge/interactive") {
            "rebase-i" // git.cc:68 GIT_REPOSITORY_STATE_REBASE_INTERACTIVE
        } else {
            "rebase-m" // git.cc:70 GIT_REPOSITORY_STATE_REBASE_MERGE
        }
    } else if exists("rebase-apply") {
        if exists("rebase-apply/rebasing") {
            "rebase" // git.cc:66 GIT_REPOSITORY_STATE_REBASE
        } else if exists("rebase-apply/applying") {
            "am" // git.cc:72 GIT_REPOSITORY_STATE_APPLY_MAILBOX
        } else {
            "am/rebase" // git.cc:74 GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE
        }
    } else if exists("MERGE_HEAD") {
        "merge" // git.cc:54 GIT_REPOSITORY_STATE_MERGE
    } else if exists("REVERT_HEAD") {
        if exists("sequencer/todo") {
            "revert-seq" // git.cc:58 GIT_REPOSITORY_STATE_REVERT_SEQUENCE
        } else {
            "revert" // git.cc:56 GIT_REPOSITORY_STATE_REVERT
        }
    } else if exists("CHERRY_PICK_HEAD") {
        if exists("sequencer/todo") {
            "cherry-seq" // git.cc:62 GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE
        } else {
            "cherry" // git.cc:60 GIT_REPOSITORY_STATE_CHERRYPICK
        }
    } else if exists("BISECT_LOG") {
        "bisect" // git.cc:64 GIT_REPOSITORY_STATE_BISECT
    } else {
        "" // git.cc:52 GIT_REPOSITORY_STATE_NONE
    };

    // git.cc:96-102 — rebase-merge carries msgnum/end, rebase-apply next/last.
    let (next, last) = if exists("rebase-merge") {
        (read1("rebase-merge/msgnum"), read1("rebase-merge/end"))
    } else if exists("rebase-apply") {
        (read1("rebase-apply/next"), read1("rebase-apply/last"))
    } else {
        (String::new(), String::new())
    };

    // git.cc:104-107 — append " next/last" only when both are present.
    if !next.is_empty() && !last.is_empty() {
        format!("{state} {next}/{last}")
    } else {
        state.to_string()
    }
}

// ---------------------------------------------------------------------------
// Counts via `git status --porcelain=v2`
// ---------------------------------------------------------------------------

/// Run the one subprocess. Output is drained on a helper thread so a huge
/// dirty tree can't deadlock on a full pipe; the caller waits at most
/// SUBPROCESS_BUDGET. Returns None on spawn failure or timeout (child is
/// killed and reaped).
fn run_porcelain(work_dir: &Path) -> Option<String> {
    let mut child = match Command::new("git")
        .arg("-C")
        .arg(work_dir)
        .args(["status", "--porcelain=v2", "--branch", "--show-stash"])
        // Match gitstatusd: never take optional locks / touch the index
        // from a prompt-driven read.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("p10k git: failed to spawn git status: {e}");
            return None;
        }
    };

    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        let _ = tx.send(out);
    });

    match rx.recv_timeout(SUBPROCESS_BUDGET) {
        Ok(out) => {
            let _ = child.wait();
            Some(out)
        }
        Err(_) => {
            tracing::debug!(
                "p10k git: git status exceeded {SUBPROCESS_BUDGET:?} in {work_dir:?}, serving cache"
            );
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// Parse `git status --porcelain=v2 --branch --show-stash` output into the
/// count fields (plus upstream/ahead/behind and commit/branch fallbacks).
///
/// Header lines:
///   `# branch.oid <oid>`            — commit (fallback if HEAD read failed)
///   `# branch.head <name>`          — `(detached)` when detached
///   `# branch.upstream <r>/<b>`     — remote_branch
///   `# branch.ab +A -B`             — ahead/behind
///   `# stash <N>`                   — stash count (--show-stash)
/// Entry lines:
///   `1 XY ...` / `2 XY ...`         — X!='.' staged++, Y!='.' unstaged++
///   `u ...`                         — conflicted++ (unmerged)
///   `? <path>`                      — untracked++
fn parse_porcelain_v2(out: &str, status: &mut GitStatus) {
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            if let Some(oid) = rest.strip_prefix("branch.oid ") {
                if status.commit.is_empty() && oid != "(initial)" {
                    status.commit = oid.to_string();
                }
            } else if let Some(head) = rest.strip_prefix("branch.head ") {
                if status.branch.is_empty() && head != "(detached)" {
                    status.branch = head.to_string();
                }
            } else if let Some(up) = rest.strip_prefix("branch.upstream ") {
                status.remote_branch = up.to_string();
            } else if let Some(ab) = rest.strip_prefix("branch.ab ") {
                for tok in ab.split_whitespace() {
                    if let Some(a) = tok.strip_prefix('+') {
                        status.ahead = a.parse().unwrap_or(0);
                    } else if let Some(b) = tok.strip_prefix('-') {
                        status.behind = b.parse().unwrap_or(0);
                    }
                }
            } else if let Some(n) = rest.strip_prefix("stash ") {
                status.stashes = n.trim().parse().unwrap_or(0);
            }
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            // XY at bytes 2..4: X = staged state, Y = worktree state.
            let bytes = line.as_bytes();
            if bytes.len() >= 4 {
                if bytes[2] != b'.' {
                    status.staged += 1;
                }
                if bytes[3] != b'.' {
                    status.unstaged += 1;
                }
            }
        } else if line.starts_with("u ") {
            status.conflicted += 1;
        } else if line.starts_with("? ") {
            status.untracked += 1;
        }
        // `! <path>` (ignored entries) are not counted — p10k doesn't use them.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_headers_and_counts() {
        let out = "\
# branch.oid 47a386b42798b5ed5d89bc617920d9152479013e
# branch.head main
# branch.upstream origin/main
# branch.ab +3 -2
# stash 1
1 .M N... 100644 100644 100644 aaaa bbbb src/lib.rs
1 M. N... 100644 100644 100644 aaaa bbbb src/exec.rs
1 MM N... 100644 100644 100644 aaaa bbbb src/hist.rs
2 R. N... 100644 100644 100644 aaaa bbbb R100 new.rs\tnew_old.rs
u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflict.rs
? untracked_a.rs
? untracked_b.rs
";
        let mut s = GitStatus::default();
        parse_porcelain_v2(out, &mut s);
        assert_eq!(s.commit, "47a386b42798b5ed5d89bc617920d9152479013e");
        assert_eq!(s.branch, "main");
        assert_eq!(s.remote_branch, "origin/main");
        assert_eq!(s.ahead, 3);
        assert_eq!(s.behind, 2);
        assert_eq!(s.stashes, 1);
        // staged: "M." + "MM" + "R." = 3; unstaged: ".M" + "MM" = 2
        assert_eq!(s.staged, 3);
        assert_eq!(s.unstaged, 2);
        assert_eq!(s.conflicted, 1);
        assert_eq!(s.untracked, 2);
    }

    #[test]
    fn porcelain_detached_and_initial() {
        let out = "\
# branch.oid (initial)
# branch.head (detached)
";
        let mut s = GitStatus::default();
        parse_porcelain_v2(out, &mut s);
        assert_eq!(s.commit, "");
        assert_eq!(s.branch, "");
        assert_eq!(s.remote_branch, "");
        assert_eq!(s.ahead, 0);
        assert_eq!(s.behind, 0);
        assert_eq!(s.stashes, 0);
    }

    #[test]
    fn porcelain_does_not_clobber_head_read() {
        // branch/commit already resolved from .git/HEAD win over headers.
        let out = "\
# branch.oid ffffffffffffffffffffffffffffffffffffffff
# branch.head porcelain-branch
";
        let mut s = GitStatus {
            branch: "head-branch".into(),
            commit: "1111111111111111111111111111111111111111".into(),
            ..GitStatus::default()
        };
        parse_porcelain_v2(out, &mut s);
        assert_eq!(s.branch, "head-branch");
        assert_eq!(s.commit, "1111111111111111111111111111111111111111");
    }

    #[test]
    fn porcelain_no_upstream_no_stash_line() {
        let out = "\
# branch.oid aaaa
# branch.head feature/x
1 A. N... 000000 100644 100644 0000 aaaa new_file.rs
";
        let mut s = GitStatus::default();
        parse_porcelain_v2(out, &mut s);
        assert_eq!(s.branch, "feature/x");
        assert_eq!(s.remote_branch, "");
        assert_eq!(s.ahead, 0);
        assert_eq!(s.behind, 0);
        assert_eq!(s.staged, 1);
        assert_eq!(s.unstaged, 0);
    }

    #[test]
    fn porcelain_ignores_ignored_entries_and_short_lines() {
        let out = "! ignored.log\n1\n\n? u.rs\n";
        let mut s = GitStatus::default();
        parse_porcelain_v2(out, &mut s);
        assert_eq!(s.staged, 0);
        assert_eq!(s.unstaged, 0);
        assert_eq!(s.untracked, 1);
    }
}
