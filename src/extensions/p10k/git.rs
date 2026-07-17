//! p10k git status backend — native replacement for gitstatusd.
//!
//! Phase-2 implementation strategy (native `.git` reader, subprocess gated):
//! - Repo discovery + branch/commit/action come from reading `.git` directly:
//!   `.git` may be a file containing `gitdir: <path>` (worktrees/submodules);
//!   `HEAD` is either a symbolic ref (`ref: refs/heads/<branch>`) or a detached
//!   commit hash; in-progress action state (rebase/merge/cherry-pick/bisect/
//!   revert/am) is detected from marker files/dirs in the gitdir, mirroring
//!   gitstatus's action names exactly (gitstatus/src/git.cc `RepoState`, which
//!   itself mirrors libgit2 `git_repository_state`). Branch tips resolve via
//!   loose refs then `packed-refs` in the common dir.
//! - NATIVE fields (no subprocess): branch, commit, action, remote_branch
//!   (from `.git/config` `branch.<name>.remote`/`branch.<name>.merge`,
//!   branch-only name per gitstatus.plugin.zsh:39,97), unstaged + conflicted
//!   (native `.git/index` v2/v3/v4 parse + lstat scan, port of gitstatus
//!   index.cc IsModified), stashes (`.git/logs/refs/stash` reflog line count —
//!   exactly what `git stash list` counts), tag (refs/tags loose scan +
//!   packed-refs peel lines, port of gitstatus tag_db.cc TagForCommit), and
//!   ahead/behind in the common case (upstream tip == HEAD tip, or no
//!   upstream ⇒ 0/0, matching gitstatusd's absent-upstream output).
//! - SUBPROCESS fields (`git status --porcelain=v2 --branch --show-stash`,
//!   ONE exec per cache miss, gated to only these fields):
//!   * staged  — requires diffing the index against the HEAD tree
//!     (gitstatus repo.cc:127-243 does `git_diff_tree_to_index`); reading the
//!     HEAD tree needs the object store (zlib-inflated loose objects and
//!     packfiles) and no inflate impl exists in-tree (Cargo.toml has zstd,
//!     not zlib). Not faked natively.
//!   * untracked — requires a faithful .gitignore engine (nested .gitignore,
//!     info/exclude, core.excludesFile, negations, dir-only patterns, `**`),
//!     which is out of scope for this file. Not faked natively.
//!   * ahead/behind when the branch and upstream tips DIVERGE — the commit
//!     count needs an object-store commit walk (same zlib wall as staged);
//!     `# branch.ab` from the porcelain fallback is kept for this case only.
//!   * everything, when the native index parse fails (unsupported version,
//!     sparse-index tree entries, oversized index, corrupt data).
//! - Tag lookup gap: loose ANNOTATED tags (a `refs/tags/x` file holding a tag
//!   object id, not the commit id) cannot be peeled without the object store
//!   and are skipped natively; packed-refs `^<oid>` peel lines cover annotated
//!   tags in the normal (packed) case. gitstatus peels via libgit2
//!   (tag_db.cc:295-330 TagHasTarget); this is a documented divergence, not
//!   an approximation.
//! - Cache: per-gitdir entry with ~2s TTL, additionally invalidated when
//!   `.git/index` or `.git/HEAD` mtime changes. The prompt is never blocked
//!   more than ~45ms: the subprocess output is read on a helper thread and
//!   awaited via `mpsc::recv_timeout`; on timeout the child is killed and the
//!   fresh NATIVE fields are returned with the stale cached subprocess-only
//!   fields merged in (or None when there is no cache yet).
//! - Perf: native path is one ~400KB index read + one lstat per index entry
//!   (3,884 entries on this repo) + a few small ref/config/log reads.
//!   Target < 5ms warm on this repo (cache hit is a map lookup; cache miss is
//!   dominated by the lstat scan, low single-digit ms warm). Indexes over
//!   NATIVE_SCAN_MAX entries skip the native scan to protect the budget.
//! - No new crate dependencies; std only.
//!
//! p10k consumes these fields as the `VCS_STATUS_*` parameters that gitstatusd
//! used to publish (p10k:3775-3783 — `$VCS_STATUS_COMMIT`,
//! `$VCS_STATUS_LOCAL_BRANCH`, `$VCS_STATUS_REMOTE_BRANCH`,
//! `$VCS_STATUS_ACTION`, `$VCS_STATUS_NUM_STAGED`, `$VCS_STATUS_NUM_UNSTAGED`,
//! `$VCS_STATUS_NUM_CONFLICTED`, `$VCS_STATUS_NUM_UNTRACKED`,
//! `$VCS_STATUS_COMMITS_AHEAD`, `$VCS_STATUS_COMMITS_BEHIND`,
//! `$VCS_STATUS_STASHES`, `$VCS_STATUS_TAG`). The tag p10k renders
//! (p10k:3959-3961) is gitstatus's EXACT match of a tag to the HEAD commit
//! (tag_db.cc:119-148 TagForCommit), not a `git describe` nearest-tag.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
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
/// we amortize the index scan + subprocess instead.
const CACHE_TTL: Duration = Duration::from_secs(2);
/// Prompt latency budget for the `git status` subprocess.
const SUBPROCESS_BUDGET: Duration = Duration::from_millis(45);
/// Native index scan cap: one lstat per entry; 20k entries ≈ ~10-20ms warm,
/// which is the most the prompt path should ever spend. Bigger indexes fall
/// back to the subprocess for unstaged/conflicted (gitstatus has the same
/// escape hatch via its dirty_max_index_size option).
const NATIVE_SCAN_MAX: usize = 20_000;

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

    // -------- native layer (no subprocess) --------
    let mut status = GitStatus::default();
    read_head(&repo, &mut status);
    status.action = repo_action(&repo.git_dir); // gitstatus git.cc:43-110 RepoState

    let cfg = GitConfig::load(&repo.common_dir.join("config"));
    let caps = RepoCaps::from_config(&cfg);

    // Upstream from branch.<name>.remote / branch.<name>.merge. REMOTE_BRANCH
    // is the branch-only name (gitstatus.plugin.zsh:39 `VCS_STATUS_REMOTE_BRANCH=master`),
    // which is what p10k compares against the local branch (p10k:3967-3968).
    let mut ab_native = false;
    if !status.branch.is_empty() {
        match upstream_of(&cfg, &status.branch) {
            Some((remote, up_branch)) => {
                status.remote_branch = up_branch.clone();
                // `remote = .` means the upstream is a local branch.
                let up_ref = if remote == "." {
                    format!("refs/heads/{up_branch}")
                } else {
                    format!("refs/remotes/{remote}/{up_branch}")
                };
                match resolve_ref(&repo.common_dir, &up_ref) {
                    Some(tip) if tip == status.commit => {
                        // Tips equal ⇒ ahead=behind=0 without any object walk.
                        ab_native = true;
                    }
                    Some(_) => {
                        // Diverged: counting commits needs an object-store
                        // walk (zlib) — keep `# branch.ab` from the porcelain
                        // fallback for this case only (see module doc).
                    }
                    None => {
                        // Upstream configured but its ref is gone (deleted
                        // remote branch). gitstatusd reports 0/0 here.
                        ab_native = true;
                    }
                }
            }
            None => ab_native = true, // no upstream ⇒ 0/0, matches gitstatusd
        }
    } else {
        ab_native = true; // detached HEAD has no upstream
    }

    // Stashes: reflog line count — `git stash list` is exactly this reflog.
    let native_stash = stash_count(&repo.common_dir);
    if let Some(n) = native_stash {
        status.stashes = n;
    }

    // Tag: exact match of refs/tags/* against the HEAD commit
    // (tag_db.cc:119-148 TagForCommit).
    if !status.commit.is_empty() {
        status.tag = tag_for_commit(&repo.common_dir, &status.commit);
    }

    // Index scan: unstaged + conflicted natively.
    let native_counts = read_index_counts(&repo, &caps);
    if let Some(ic) = &native_counts {
        status.unstaged = ic.unstaged;
        status.conflicted = ic.conflicted;
    }

    // -------- gated subprocess (staged/untracked always; the rest only when
    // the native layer could not produce them — see module doc) --------
    match run_porcelain(&repo.work_dir) {
        Some(out) => {
            let mut sub = GitStatus::default();
            parse_porcelain_v2(&out, &mut sub);
            status.staged = sub.staged;
            status.untracked = sub.untracked;
            if native_counts.is_none() {
                status.unstaged = sub.unstaged;
                status.conflicted = sub.conflicted;
            }
            if !ab_native {
                status.ahead = sub.ahead;
                status.behind = sub.behind;
            }
            if native_stash.is_none() {
                status.stashes = sub.stashes; // --show-stash fallback
            }
            if status.commit.is_empty() {
                status.commit = sub.commit;
            }
            if status.branch.is_empty() {
                status.branch = sub.branch;
            }
            if status.remote_branch.is_empty() && !sub.remote_branch.is_empty() {
                // `# branch.upstream` is `<remote>/<branch>`; strip the remote
                // (remote names cannot contain '/') to the branch-only shape.
                status.remote_branch = match sub.remote_branch.split_once('/') {
                    Some((_, b)) => b.to_string(),
                    None => sub.remote_branch,
                };
            }
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
        // Subprocess overran the budget: return the FRESH native fields with
        // the stale cached subprocess-only fields merged in (slightly-old
        // staged/untracked beat a blocked prompt); None when no cache yet.
        None => {
            if let Ok(map) = cache().lock() {
                if let Some(e) = map.get(&repo.git_dir) {
                    status.staged = e.status.staged;
                    status.untracked = e.status.untracked;
                    if native_counts.is_none() {
                        status.unstaged = e.status.unstaged;
                        status.conflicted = e.status.conflicted;
                    }
                    if !ab_native {
                        status.ahead = e.status.ahead;
                        status.behind = e.status.behind;
                    }
                    if native_stash.is_none() {
                        status.stashes = e.status.stashes;
                    }
                    return Some(status);
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
    /// Directory containing refs/, packed-refs, config, logs/refs/stash
    /// (shared across worktrees).
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
// .git/config (minimal parser: sections, subsections, key=value)
// ---------------------------------------------------------------------------

/// Minimal `.git/config` reader for `branch.<name>.remote/merge`,
/// `core.filemode/symlinks`, and `extensions.objectformat`. Sections and key
/// names are case-insensitive, subsections case-sensitive (git-config(1)).
/// Not supported (all rare for these keys; missing keys degrade to defaults
/// or to the porcelain fallback): `include.path`/`includeIf`, multi-line
/// quoted values, `\n`-escapes inside values.
struct GitConfig {
    /// key = "<section-lower>\0<subsection-verbatim>\0<key-lower>"
    map: HashMap<String, String>,
}

impl GitConfig {
    fn load(path: &Path) -> GitConfig {
        let text = fs::read_to_string(path).unwrap_or_default();
        GitConfig::parse(&text)
    }

    fn parse(text: &str) -> GitConfig {
        let mut map = HashMap::new();
        let mut sect = String::new();
        let mut sub = String::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(body) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                // `[section]` or `[section "sub\"section"]`
                match body.split_once(char::is_whitespace) {
                    Some((s, rest)) => {
                        sect = s.to_ascii_lowercase();
                        let rest = rest.trim();
                        sub = rest
                            .strip_prefix('"')
                            .and_then(|r| r.strip_suffix('"'))
                            .map(|r| r.replace("\\\\", "\\").replace("\\\"", "\""))
                            .unwrap_or_else(|| rest.to_string());
                    }
                    None => {
                        sect = body.to_ascii_lowercase();
                        sub = String::new();
                    }
                }
                continue;
            }
            // `key = value`, or bare `key` (implicit boolean true).
            let (key, mut val) = match line.split_once('=') {
                Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim().to_string()),
                None => (line.to_ascii_lowercase(), "true".to_string()),
            };
            // Strip an unquoted trailing comment, then surrounding quotes.
            if !val.starts_with('"') {
                if let Some(i) = val.find(['#', ';']) {
                    val.truncate(i);
                    val = val.trim_end().to_string();
                }
            } else if let Some(stripped) = val
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
            {
                val = stripped.replace("\\\\", "\\").replace("\\\"", "\"");
            }
            map.insert(format!("{sect}\0{sub}\0{key}"), val);
        }
        GitConfig { map }
    }

    fn get(&self, section: &str, subsection: &str, key: &str) -> Option<&str> {
        self.map
            .get(&format!("{section}\0{subsection}\0{key}"))
            .map(String::as_str)
    }

    fn get_bool(&self, section: &str, key: &str, default: bool) -> bool {
        match self.get(section, "", key) {
            Some(v) => matches!(
                v.to_ascii_lowercase().as_str(),
                "true" | "yes" | "on" | "1"
            ),
            None => default,
        }
    }
}

/// `branch.<name>.remote` + `branch.<name>.merge` → (remote, upstream branch
/// name). The merge value is a full ref (`refs/heads/main`); the returned
/// name is branch-only, matching gitstatusd's VCS_STATUS_REMOTE_BRANCH shape
/// (gitstatus.plugin.zsh:39,97).
fn upstream_of(cfg: &GitConfig, branch: &str) -> Option<(String, String)> {
    let remote = cfg.get("branch", branch, "remote")?;
    let merge = cfg.get("branch", branch, "merge")?;
    let name = merge.strip_prefix("refs/heads/").unwrap_or(merge);
    if remote.is_empty() || name.is_empty() {
        return None;
    }
    Some((remote.to_string(), name.to_string()))
}

/// Port of gitstatus RepoCaps (index.cc:308-318): repository capabilities
/// that steer the lstat comparison. libgit2 defaults both to true on unix;
/// `git clone`/`git init` write them explicitly.
struct RepoCaps {
    trust_filemode: bool, // core.filemode  (index.cc:309 is_filemode_trustworthy)
    has_symlinks: bool,   // core.symlinks  (index.cc:310 index_supports_symlinks)
    hash_len: usize,      // 20 (sha1) / 32 (extensions.objectformat = sha256)
}

impl RepoCaps {
    fn from_config(cfg: &GitConfig) -> RepoCaps {
        RepoCaps {
            trust_filemode: cfg.get_bool("core", "filemode", true),
            has_symlinks: cfg.get_bool("core", "symlinks", true),
            hash_len: match cfg.get("extensions", "", "objectformat") {
                Some("sha256") => 32,
                _ => 20,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// .git/index (v2/v3/v4) → unstaged + conflicted
// ---------------------------------------------------------------------------

struct IndexCounts {
    unstaged: i64,
    conflicted: i64,
}

fn be32(data: &[u8], pos: usize) -> Option<u32> {
    data.get(pos..pos + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn be16(data: &[u8], pos: usize) -> Option<u16> {
    data.get(pos..pos + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
}

/// git varint.c decode_varint — "offset encoding": 7 bits per byte, MSB is
/// the continuation bit, and each continuation adds 1 before shifting so
/// that multi-byte encodings have no redundant forms.
fn decode_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut c = *data.get(*pos)?;
    *pos += 1;
    let mut val = u64::from(c & 0x7f);
    let mut rounds = 0;
    while c & 0x80 != 0 {
        rounds += 1;
        if rounds > 9 {
            return None; // > 63 significant bits ⇒ corrupt
        }
        c = *data.get(*pos)?;
        *pos += 1;
        val = val.checked_add(1)?.checked_shl(7)? | u64::from(c & 0x7f);
    }
    Some(val)
}

/// The per-entry stat fields the modified check needs (gitformat-index.txt
/// entry layout; offsets relative to the entry start):
///   0 ctime-sec  4 ctime-nsec  8 mtime-sec  12 mtime-nsec  16 dev  20 ino
///   24 mode  28 uid  32 gid  36 file-size  (all u32 big-endian, truncated)
///   40 .. 40+H  object id (H = 20 sha1 / 32 sha256)
///   40+H  flags u16be: bit15 assume-valid, bit14 extended (v3+),
///         bits13-12 stage, bits11-0 name length (0xFFF when longer)
///   [+2 extended flags u16be when the extended bit is set (v3+):
///         bit14 skip-worktree, bit13 intent-to-add]
///   then the path: v2/v3 NUL-terminated + NUL padding to a multiple of 8
///   bytes from the entry start (1-8 NULs); v4 prefix-compressed — varint N
///   (bytes to strip from the END of the previous path) + NUL-terminated
///   suffix, no padding.
struct EntryStat {
    mode: u32,
    ino: u32,
    size: u32,
    mt_s: u32,
    mt_ns: u32,
}

const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;
const S_IFDIR: u32 = 0o040000;

/// Port of gitstatus IsModified (index.cc:71-104) — decides whether a
/// stage-0 index entry differs from its worktree file. gitstatus compares
/// exactly ino / stage / fsize / mtime / mode (index.cc:93-99); dev, uid,
/// gid and ctime are NOT compared.
fn is_modified(e: &EntryStat, st: &fs::Metadata, caps: &RepoCaps) -> bool {
    use std::os::unix::fs::MetadataExt;
    // index.cc:71-83 — normalize the worktree mode before comparing.
    let st_mode = st.mode();
    let mode = if st_mode & S_IFMT == S_IFREG {
        if !caps.has_symlinks && e.mode & S_IFMT == S_IFLNK {
            e.mode
        } else if !caps.trust_filemode {
            e.mode
        } else {
            S_IFREG | if st_mode & 0o100 != 0 { 0o755 } else { 0o644 }
        }
    } else {
        st_mode & S_IFMT
    };
    // index.cc:93 — ino: 0 in the index matches anything; else compare the
    // u32-truncated st_ino exactly as C does.
    if e.ino != 0 && e.ino != st.ino() as u32 {
        return true;
    }
    // index.cc:97 — fsize. NOTE racy-git: git smudges racily-clean entries
    // to file_size 0 on index write, forcing a content check; gitstatus (and
    // git) would then hash the file. We instead COUNT the entry as unstaged
    // per the racy-git convention — a just-touched-but-identical file may
    // show unstaged for one prompt until the next `git status`/`git add`
    // rewrites the index. Never wrong about truly-dirty files.
    if u64::from(e.size) != st.size() {
        return true;
    }
    // index.cc:61-69 MTimeEq — seconds must match; nanoseconds match, or the
    // index stores 0 nsec (git built without USE_NSEC — the GITSTATUS_ZERO_NSEC
    // build flag; applied unconditionally here since a 0-nsec index entry vs
    // a real-nsec filesystem is the common macOS/Linux packaged-git case).
    if e.mt_s as i64 != st.mtime() || !(e.mt_ns as i64 == st.mtime_nsec() || e.mt_ns == 0) {
        return true;
    }
    // index.cc:99 — mode.
    if e.mode != mode {
        return true;
    }
    false
}

/// Parse `.git/index` and lstat each stage-0 entry: unstaged = entries whose
/// worktree file is missing/unreadable/modified (index.cc:191-199 StatFiles
/// + IsModified), conflicted = distinct paths with stage > 0. Returns None
/// when the native parse cannot proceed (unsupported version, sparse-index
/// tree entries, oversized index, corrupt data) — the caller then takes both
/// counts from the porcelain subprocess. The trailing checksum is NOT
/// verified (read-only prompt path; no SHA-1 impl in-tree — worst case a
/// torn index yields wrong counts for one 2s cache window).
fn read_index_counts(repo: &Repo, caps: &RepoCaps) -> Option<IndexCounts> {
    let data = fs::read(repo.git_dir.join("index")).ok()?;
    // Header (12 bytes): "DIRC", version u32be, entry count u32be; the file
    // ends with a hash-len checksum trailer.
    if data.len() < 12 + caps.hash_len || &data[0..4] != b"DIRC" {
        tracing::debug!("p10k git: index missing/short/bad magic in {:?}", repo.git_dir);
        return None;
    }
    let version = be32(&data, 4)?;
    if !(2..=4).contains(&version) {
        tracing::debug!("p10k git: unsupported index version {version}");
        return None;
    }
    let nentries = be32(&data, 8)? as usize;
    if nentries > NATIVE_SCAN_MAX {
        tracing::debug!("p10k git: index has {nentries} entries > {NATIVE_SCAN_MAX}, deferring to subprocess");
        return None;
    }

    let mut pos = 12usize;
    let mut prev_path: Vec<u8> = Vec::new(); // v4 prefix compression state
    let mut last_conflict: Vec<u8> = Vec::new();
    let mut unstaged = 0i64;
    let mut conflicted = 0i64;

    for _ in 0..nentries {
        let start = pos;
        // Fixed stat area (40) + oid + flags.
        if pos + 40 + caps.hash_len + 2 > data.len() {
            return None;
        }
        let e = EntryStat {
            mt_s: be32(&data, pos + 8)?,
            mt_ns: be32(&data, pos + 12)?,
            ino: be32(&data, pos + 20)?,
            mode: be32(&data, pos + 24)?,
            size: be32(&data, pos + 36)?,
        };
        let flags = be16(&data, pos + 40 + caps.hash_len)?;
        pos += 40 + caps.hash_len + 2;

        let assume_valid = flags & 0x8000 != 0;
        let extended = flags & 0x4000 != 0;
        let stage = (flags >> 12) & 0x3;
        let name_len = (flags & 0xFFF) as usize;

        let mut skip_worktree = false;
        let mut intent_to_add = false;
        if extended {
            if version < 3 {
                return None; // extended bit is invalid in v2 ⇒ corrupt
            }
            let ext = be16(&data, pos)?;
            pos += 2;
            skip_worktree = ext & 0x4000 != 0;
            intent_to_add = ext & 0x2000 != 0;
        }

        // Path.
        if version == 4 {
            let strip = decode_varint(&data, &mut pos)? as usize;
            if strip > prev_path.len() {
                return None;
            }
            let keep = prev_path.len() - strip;
            prev_path.truncate(keep);
            let nul = data.get(pos..)?.iter().position(|&b| b == 0)?;
            prev_path.extend_from_slice(&data[pos..pos + nul]);
            pos += nul + 1; // v4: no padding
        } else {
            let actual_len = if name_len < 0xFFF {
                name_len
            } else {
                data.get(pos..)?.iter().position(|&b| b == 0)?
            };
            prev_path.clear();
            prev_path.extend_from_slice(data.get(pos..pos + actual_len)?);
            // v2/v3 entry size: (fixed + namelen + 8) & !7 — NUL padding to a
            // multiple of 8 from the entry start, ≥ 1 NUL (read-cache.c
            // ondisk_ce_size).
            let fixed = pos - start;
            pos = start + ((fixed + actual_len + 8) & !7);
            if pos > data.len() {
                return None;
            }
        }

        // ---- evaluate the entry ----
        if stage != 0 {
            // Unmerged: 2-3 entries per path (stages 1/2/3, adjacent since
            // the index is name-then-stage sorted); count the PATH once,
            // matching one `u` line per path in porcelain v2.
            if prev_path != last_conflict {
                conflicted += 1;
                last_conflict.clear();
                last_conflict.extend_from_slice(&prev_path);
            }
            continue;
        }
        if skip_worktree || assume_valid {
            // git status ignores skip-worktree entries; assume-valid
            // (`git update-index --assume-unchanged`) means "trust the index"
            // — no stat, matching git.
            continue;
        }
        if intent_to_add {
            // `git add -N`: porcelain v2 reports `1 .A` — an unstaged add.
            unstaged += 1;
            continue;
        }
        if e.mode & S_IFMT == S_IFDIR {
            // Sparse-index tree entry (extensions.sparseIndex): expanding it
            // needs the object store — defer everything to the subprocess.
            tracing::debug!("p10k git: sparse index in {:?}, deferring to subprocess", repo.git_dir);
            return None;
        }
        let full = repo.work_dir.join(OsStr::from_bytes(&prev_path));
        match fs::symlink_metadata(&full) {
            // index.cc:195 — ENOENT ⇒ deleted, other errors ⇒ unreadable;
            // both are dirty candidates.
            Err(_) => unstaged += 1,
            Ok(st) => {
                if is_modified(&e, &st, caps) {
                    unstaged += 1;
                }
            }
        }
    }
    // Extensions (TREE, REUC, UNTR, ...) follow the entries; none are needed
    // for these two counts, so parsing stops here.

    Some(IndexCounts {
        unstaged,
        conflicted,
    })
}

// ---------------------------------------------------------------------------
// Stashes — .git/logs/refs/stash reflog
// ---------------------------------------------------------------------------

/// Stash count = line count of the stash reflog; `git stash list` renders
/// exactly this reflog, one line per stash. The file lives in the COMMON dir
/// (reflogs other than HEAD are shared across worktrees). Absent file = no
/// stashes (git deletes it when the last stash is dropped). None only on a
/// real read error (the caller then falls back to porcelain `# stash`).
fn stash_count(common_dir: &Path) -> Option<i64> {
    match fs::read(common_dir.join("logs/refs/stash")) {
        Ok(bytes) => Some(bytes.iter().filter(|&&b| b == b'\n').count() as i64),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(0),
        Err(e) => {
            tracing::debug!("p10k git: stash reflog unreadable: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tag — port of gitstatus TagDb::TagForCommit (tag_db.cc:119-148)
// ---------------------------------------------------------------------------

/// The tag p10k shows for HEAD: the lexicographically GREATEST tag name whose
/// (peeled) target equals the HEAD commit (tag_db.cc:130 `if (res < tag ...)`
/// and :143 — max over loose and packed matches). "" when no tag points at
/// HEAD. Loose tags shadow same-named packed tags (tag_db.cc:135,143
/// IsLooseTag). Loose ANNOTATED tags (file holds a tag object id) cannot be
/// peeled without the object store and are skipped — see module doc.
fn tag_for_commit(common_dir: &Path, commit: &str) -> String {
    let mut best = String::new();
    let mut loose_names: Vec<String> = Vec::new();

    // Loose tags: flat scan of refs/tags, mirroring tag_db.cc:150-162
    // ReadLooseTags (non-recursive; nested names like refs/tags/foo/bar are
    // missed — gitstatus has the same TODO at tag_db.cc:158).
    if let Ok(rd) = fs::read_dir(common_dir.join("refs/tags")) {
        for entry in rd.flatten() {
            let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
            if !is_file {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if let Ok(oid) = fs::read_to_string(entry.path()) {
                // Lightweight tag: the file IS the commit id. An annotated
                // tag's file holds the tag object id ⇒ no match here (skipped
                // per module doc).
                if oid.trim() == commit && best < name {
                    best = name.clone();
                }
            }
            loose_names.push(name);
        }
    }

    // Packed tags. tag_db.cc:228-231: without a `#` header line the refs
    // cannot be assumed fully-peeled — drop all packed tags. tag_db.cc:236-238:
    // a header missing " fully-peeled"/" sorted" only warns and continues.
    let Ok(packed) = fs::read_to_string(common_dir.join("packed-refs")) else {
        return best;
    };
    if !packed.starts_with('#') {
        tracing::warn!("p10k git: packed-refs doesn't have a header. Won't resolve packed tags.");
        return best;
    }
    if let Some(header) = packed.lines().next() {
        if !header.contains(" fully-peeled") || !header.contains(" sorted") {
            tracing::warn!("p10k git: packed-refs has unexpected header."); // tag_db.cc:237
        }
    }
    let mut lines = packed.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        let Some((oid, refname)) = line.split_once(' ') else {
            continue;
        };
        // A following `^<oid>` line is the peeled target of an annotated tag
        // (tag_db.cc:256-263 replaces the id with the peeled one).
        let effective = match lines.peek() {
            Some(peel) if peel.starts_with('^') => lines
                .next()
                .and_then(|p| p.strip_prefix('^'))
                .unwrap_or(oid),
            _ => oid,
        };
        let Some(name) = refname.strip_prefix("refs/tags/") else {
            continue;
        };
        if effective == commit
            && !loose_names.iter().any(|l| l == name) // loose shadows packed
            && best.as_str() < name
        {
            best = name.to_string();
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Gated subprocess: `git status --porcelain=v2` for staged / untracked
// (+ ahead-behind on divergence, + everything on native-parse failure)
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

    // ---- porcelain fallback parser (unchanged behavior) ----

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

    // ---- native index parse ----

    fn caps() -> RepoCaps {
        RepoCaps {
            trust_filemode: true,
            has_symlinks: true,
            hash_len: 20,
        }
    }

    /// Build one v2 on-disk entry: 40-byte stat area + 20-byte oid + flags +
    /// NUL-padded path (entry size = (62 + len + 8) & !7).
    fn v2_entry(path: &[u8], mode: u32, ino: u32, size: u32, mt_s: u32, mt_ns: u32, stage: u16) -> Vec<u8> {
        let mut e = Vec::new();
        for v in [0u32, 0, mt_s, mt_ns, 0, ino, mode, 0, 0, size] {
            e.extend_from_slice(&v.to_be_bytes());
        }
        e.extend_from_slice(&[0u8; 20]); // oid (not read by the parser)
        let flags: u16 = ((stage & 0x3) << 12) | (path.len().min(0xFFF) as u16);
        e.extend_from_slice(&flags.to_be_bytes());
        e.extend_from_slice(path);
        let total = (62 + path.len() + 8) & !7;
        e.resize(total, 0);
        e
    }

    fn v2_index(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut d = b"DIRC".to_vec();
        d.extend_from_slice(&2u32.to_be_bytes());
        d.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for e in entries {
            d.extend_from_slice(e);
        }
        d.extend_from_slice(&[0u8; 20]); // checksum trailer (not verified)
        d
    }

    fn temp_repo(index: &[u8]) -> (tempfile::TempDir, Repo) {
        let td = tempfile::tempdir().expect("tempdir");
        let git_dir = td.path().join(".git");
        fs::create_dir_all(&git_dir).expect("mkdir .git");
        fs::write(git_dir.join("index"), index).expect("write index");
        let repo = Repo {
            common_dir: git_dir.clone(),
            git_dir,
            work_dir: td.path().to_path_buf(),
        };
        (td, repo)
    }

    #[test]
    fn index_header_rejects_bad_magic_and_version() {
        // Bad magic on a fixture byte literal.
        let mut bad = b"XDIR".to_vec();
        bad.extend_from_slice(&2u32.to_be_bytes());
        bad.extend_from_slice(&0u32.to_be_bytes());
        bad.extend_from_slice(&[0u8; 20]);
        let (_td, repo) = temp_repo(&bad);
        assert!(read_index_counts(&repo, &caps()).is_none());

        // Unsupported version 5.
        let mut v5 = b"DIRC".to_vec();
        v5.extend_from_slice(&5u32.to_be_bytes());
        v5.extend_from_slice(&0u32.to_be_bytes());
        v5.extend_from_slice(&[0u8; 20]);
        let (_td, repo) = temp_repo(&v5);
        assert!(read_index_counts(&repo, &caps()).is_none());
    }

    #[test]
    fn index_v2_empty_and_conflicts() {
        // Header-only v2 index (fixture byte literal): 0 entries, 0 counts.
        let (_td, repo) = temp_repo(&v2_index(&[]));
        let ic = read_index_counts(&repo, &caps()).expect("empty index parses");
        assert_eq!(ic.unstaged, 0);
        assert_eq!(ic.conflicted, 0);

        // Stages 1+2 for one path = ONE conflicted path (matches one `u`
        // porcelain line), no lstat performed for unmerged entries.
        let e1 = v2_entry(b"c.rs", 0o100644, 1, 5, 1, 0, 1);
        let e2 = v2_entry(b"c.rs", 0o100644, 1, 5, 1, 0, 2);
        let (_td, repo) = temp_repo(&v2_index(&[e1, e2]));
        let ic = read_index_counts(&repo, &caps()).expect("parses");
        assert_eq!(ic.conflicted, 1);
        assert_eq!(ic.unstaged, 0);
    }

    #[test]
    fn index_v2_lstat_clean_deleted_modified() {
        use std::os::unix::fs::MetadataExt;
        let td = tempfile::tempdir().expect("tempdir");
        let f = td.path().join("a.rs");
        fs::write(&f, b"x").expect("write file");
        let st = fs::symlink_metadata(&f).expect("stat");
        let mode = S_IFREG | if st.mode() & 0o100 != 0 { 0o755 } else { 0o644 };

        // Entry stat-matching the real file → clean.
        let clean = v2_entry(
            b"a.rs",
            mode,
            st.ino() as u32,
            st.size() as u32,
            st.mtime() as u32,
            st.mtime_nsec() as u32,
            0,
        );
        // Same but wrong size → modified (index.cc:97).
        let dirty = v2_entry(
            b"b.rs",
            mode,
            0,
            st.size() as u32 + 7,
            st.mtime() as u32,
            0,
            0,
        );
        fs::write(td.path().join("b.rs"), b"x").expect("write b");
        // Entry whose worktree file does not exist → deleted (index.cc:195).
        let gone = v2_entry(b"gone.rs", mode, 0, 1, 1, 0, 0);

        let git_dir = td.path().join(".git");
        fs::create_dir_all(&git_dir).expect("mkdir");
        fs::write(git_dir.join("index"), v2_index(&[clean, dirty, gone])).expect("write index");
        let repo = Repo {
            common_dir: git_dir.clone(),
            git_dir,
            work_dir: td.path().to_path_buf(),
        };
        let ic = read_index_counts(&repo, &caps()).expect("parses");
        assert_eq!(ic.unstaged, 2); // dirty + gone; clean not counted
        assert_eq!(ic.conflicted, 0);
    }

    #[test]
    fn varint_offset_encoding() {
        // git varint.c: 0x7f → 127; 0x80 0x01 → ((0+1)<<7)|1 = 129.
        let mut p = 0;
        assert_eq!(decode_varint(&[0x7f], &mut p), Some(127));
        let mut p = 0;
        assert_eq!(decode_varint(&[0x80, 0x01], &mut p), Some(129));
        assert_eq!(p, 2);
        // Truncated continuation → None.
        let mut p = 0;
        assert_eq!(decode_varint(&[0x80], &mut p), None);
    }

    // ---- packed-refs tags + stash log + config ----

    const C: &str = "47a386b42798b5ed5d89bc617920d9152479013e";

    #[test]
    fn packed_refs_tag_peel_and_max_name() {
        let td = tempfile::tempdir().expect("tempdir");
        // Annotated v1.0.0 peels to C; lightweight v0.9.0 points at C
        // directly; v0.5.0 points elsewhere.
        let packed = format!(
            "# pack-refs with: peeled fully-peeled sorted \n\
             aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa refs/tags/v0.5.0\n\
             {C} refs/tags/v0.9.0\n\
             bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb refs/tags/v1.0.0\n\
             ^{C}\n"
        );
        fs::write(td.path().join("packed-refs"), packed).expect("write");
        // Lexicographic max of the two matches (tag_db.cc:130,143).
        assert_eq!(tag_for_commit(td.path(), C), "v1.0.0");

        // Loose lightweight tag beats packed by name order and shadows a
        // same-named packed entry.
        let tags = td.path().join("refs/tags");
        fs::create_dir_all(&tags).expect("mkdir");
        fs::write(tags.join("v2.0"), format!("{C}\n")).expect("write");
        assert_eq!(tag_for_commit(td.path(), C), "v2.0");
    }

    #[test]
    fn packed_refs_without_header_drops_packed_tags() {
        // tag_db.cc:228-231 — no `#` header ⇒ packed tags unusable.
        let td = tempfile::tempdir().expect("tempdir");
        fs::write(
            td.path().join("packed-refs"),
            format!("{C} refs/tags/v9.9.9\n"),
        )
        .expect("write");
        assert_eq!(tag_for_commit(td.path(), C), "");
    }

    #[test]
    fn stash_log_count() {
        let td = tempfile::tempdir().expect("tempdir");
        assert_eq!(stash_count(td.path()), Some(0)); // absent = 0, exact
        let logs = td.path().join("logs/refs");
        fs::create_dir_all(&logs).expect("mkdir");
        fs::write(
            logs.join("stash"),
            "0000 1111 user <u@example.com> 1 +0000\tWIP one\n\
             1111 2222 user <u@example.com> 2 +0000\tWIP two\n",
        )
        .expect("write");
        assert_eq!(stash_count(td.path()), Some(2));
    }

    #[test]
    fn config_upstream_and_caps() {
        let cfg = GitConfig::parse(
            "[core]\n\
             \tfilemode = false\n\
             \tsymlinks = true\n\
             [extensions]\n\
             \tobjectformat = sha256\n\
             [branch \"feature/x\"]\n\
             \tremote = origin\n\
             \tmerge = refs/heads/feature/x\n\
             [branch \"local\"]\n\
             \tremote = .\n\
             \tmerge = refs/heads/main\n",
        );
        assert_eq!(
            upstream_of(&cfg, "feature/x"),
            Some(("origin".to_string(), "feature/x".to_string()))
        );
        assert_eq!(
            upstream_of(&cfg, "local"),
            Some((".".to_string(), "main".to_string()))
        );
        assert_eq!(upstream_of(&cfg, "nope"), None);
        let caps = RepoCaps::from_config(&cfg);
        assert!(!caps.trust_filemode);
        assert!(caps.has_symlinks);
        assert_eq!(caps.hash_len, 32);
    }
}
