//! p10k `vcs` segment — the non-git backends (hg, svn).
//!
//! Upstream (`~/.zinit/plugins/romkatv---powerlevel10k/internal/p10k.zsh`)
//! renders git through gitstatus and hands every other backend in
//! `POWERLEVEL9K_VCS_BACKENDS` to zsh's `vcs_info` (`prompt_vcs`,
//! p10k:4176-4208) configured by `_p9k_vcs_info_init` (p10k:3765-3808) and
//! the `+vi-*` hooks (p10k:3681-3763). The engine does not load
//! `vcs_info`; this module is the equivalent logic, ported from the zsh
//! sources it replaces (`Functions/VCS_Info/`: `vcs_info`,
//! `Backends/VCS_INFO_{detect,get_data}_{hg,svn}`, `VCS_INFO_formats`,
//! `VCS_INFO_bydir_detect`, `VCS_INFO_set-patch-format`).
//!
//! Effective `vcs_info` configuration (all from `_p9k_vcs_info_init`):
//! - `formats`          `<prefix>%b%c%u%m` (hg), `<prefix>%c%u` (svn);
//!   `<prefix>` = `VCS_COMMIT_ICON%0.<CHANGESET_HASH_LENGTH>i ` when
//!   `SHOW_CHANGESET`.
//! - `actionformats`    `%b %F{VCS_ACTIONFORMAT_FOREGROUND}| %a%f`
//! - `stagedstr` / `unstagedstr`  ` <VCS_STAGED_ICON>` / ` <VCS_UNSTAGED_ICON>`
//! - hg `branchformat`  `<VCS_BRANCH_ICON>%b` (`%b` with HIDE_BRANCH_ICON),
//!   `get-revision` and `get-bookmarks` on, check-for-changes on.
//! - hooks              `VCS_HG_HOOKS` / `VCS_SVN_HOOKS`; the built-ins
//!   `vcs-detect-changes` and `svn-detect-changes` are implemented
//!   natively. A user-defined `+vi-*` hook name is not run (debug-logged).
//!
//! Segment state follows `prompt_vcs`: `VCS_WORKDIR_DIRTY` -> MODIFIED,
//! else `VCS_WORKDIR_HALF_DIRTY` -> UNTRACKED, else CLEAN; an empty
//! message (e.g. a clean svn tree) hides the segment.

use crate::extensions::p10k::config::{p9k_global, p9k_param};
use crate::extensions::p10k::render::Segment;
use crate::extensions::p10k::segments_core::vcs_state_default_bg;
use crate::extensions::p10k::segments_sys::cmd_on_path;
use crate::extensions::p10k::shared::{
    apply_visual_identifier, color1, global_bool, global_int, seg_icon,
};
use crate::ported::params::{getaparam, getsparam};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hard latency budget per tool invocation; the child is killed on overrun.
const TOOL_BUDGET: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------
// Subprocess
// ---------------------------------------------------------------------

struct ToolOut {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run `bin args` in `dir` with extra environment, capturing stdout and
/// stderr separately. `None` on spawn failure or budget overrun.
fn run_tool(bin: &Path, args: &[&str], dir: &Path, env: &[(&str, &str)]) -> Option<ToolOut> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let (tx, rx) = std::sync::mpsc::channel::<(bool, Vec<u8>)>();
    let tx_err = tx.clone();
    crate::signal_thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send((true, buf));
    });
    crate::signal_thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        let _ = tx_err.send((false, buf));
    });
    let deadline = Instant::now() + TOOL_BUDGET;
    let (mut so, mut se) = (None, None);
    while so.is_none() || se.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((true, b)) => so = Some(b),
            Ok((false, b)) => se = Some(b),
            Err(_) => {
                tracing::debug!(target: "p10k", bin = %bin.display(), "vcs tool exceeded budget — killed");
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let code = child.wait().ok()?.code();
    Some(ToolOut {
        code,
        stdout: String::from_utf8_lossy(&so?).into_owned(),
        stderr: String::from_utf8_lossy(&se?).into_owned(),
    })
}

// ---------------------------------------------------------------------
// Detection — VCS_INFO_bydir_detect
// ---------------------------------------------------------------------

/// Walk from `start` towards `/` (exclusive) for a directory holding
/// `dirname/` that contains at least one of `need` — the
/// `vcs_comm[detect_need_file]` form of `VCS_INFO_bydir_detect`. An
/// unreadable ancestor aborts the walk. Returns the repo base directory.
fn bydir_detect(start: &Path, dirname: &str, need: &[&str]) -> Option<PathBuf> {
    let mut base = start.to_path_buf();
    while base != Path::new("/") {
        std::fs::read_dir(&base).ok()?; // `[[ -r ${basedir} ]] || return 1`
        let marker = base.join(dirname);
        if marker.is_dir() && need.iter().any(|f| marker.join(f).exists()) {
            return Some(base);
        }
        base = base.parent()?.to_path_buf();
    }
    None
}

/// `${PWD:P}` — the physical working directory.
fn physical_cwd() -> PathBuf {
    let pwd = getsparam("PWD").filter(|p| !p.is_empty()).map(PathBuf::from);
    pwd.and_then(|p| p.canonicalize().ok())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"))
}

// ---------------------------------------------------------------------
// Configuration snapshot
// ---------------------------------------------------------------------

struct Icons {
    branch: String,
    commit: String,
    staged: String,
    unstaged: String,
    untracked: String,
    bookmark: String,
}

impl Icons {
    /// `_p9k_get_icon '' KEY` / `print_icon KEY` — stateless icon lookup.
    fn load() -> Self {
        let get = |key: &str| seg_icon("vcs", None, key);
        Icons {
            branch: get("VCS_BRANCH_ICON"),
            commit: get("VCS_COMMIT_ICON"),
            staged: get("VCS_STAGED_ICON"),
            unstaged: get("VCS_UNSTAGED_ICON"),
            untracked: get("VCS_UNTRACKED_ICON"),
            bookmark: get("VCS_BOOKMARK_ICON"),
        }
    }
}

struct Opts<'a> {
    icons: &'a Icons,
    hooks: &'a [String],
    show_changeset: bool,
    hash_len: usize,
    hide_branch_icon: bool,
    action_fg: &'a str,
}

/// `POWERLEVEL9K_<name>` as an array with a declared default (p10k:7744).
/// A set-but-empty array disables every hook, so it must not fall back.
fn hook_list(name: &str, default: &[&str]) -> Vec<String> {
    let full = format!("POWERLEVEL9K_{name}");
    getaparam(&full)
        .or_else(|| getsparam(&full).map(|s| vec![s]))
        .unwrap_or_else(|| default.iter().map(|s| s.to_string()).collect())
}

/// `%0.<N>i` — zformat truncation of the revision to `N` bytes
/// (Src/Modules/zutil.c, `smax`).
fn truncate_bytes(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ---------------------------------------------------------------------
// Message assembly (VCS_INFO_formats + the +vi- hooks)
// ---------------------------------------------------------------------

/// What `vcs_info` leaves behind for `prompt_vcs`.
#[derive(Debug, PartialEq)]
struct Message {
    text: String,
    dirty: bool,
    half_dirty: bool,
    icon_key: Option<&'static str>,
}

/// `svn status` first-column classes consumed by `+vi-svn-detect-changes`.
#[derive(Debug, Default, PartialEq)]
struct SvnFlags {
    untracked: bool,
    modified: bool,
    added: bool,
}

/// Run the configured hooks over the staged/unstaged strings, in order
/// (p10k:3716-3748). Returns (dirty, half_dirty, visual-identifier key).
fn run_hooks(
    vcs: &str,
    o: &Opts,
    staged: &mut String,
    unstaged: &mut String,
    svn: &SvnFlags,
) -> (bool, bool, Option<&'static str>) {
    let (mut dirty, mut half, mut icon_key) = (false, false, None);
    for hook in o.hooks {
        match hook.as_str() {
            "vcs-detect-changes" => {
                icon_key = match vcs {
                    "hg" => Some("VCS_HG_ICON"),
                    "svn" => Some("VCS_SVN_ICON"),
                    _ => None,
                };
                dirty = !staged.is_empty() || !unstaged.is_empty();
            }
            "svn-detect-changes" if vcs == "svn" => {
                if svn.untracked {
                    unstaged.push_str(&format!(" {}", o.icons.untracked));
                    half = true;
                }
                if svn.modified {
                    unstaged.push_str(&format!(" {}", o.icons.unstaged));
                    dirty = true;
                }
                if svn.added {
                    staged.push_str(&format!(" {}", o.icons.staged));
                    dirty = true;
                }
            }
            "svn-detect-changes" => {}
            other => tracing::debug!(target: "p10k", hook = other, "vcs hook not implemented natively"),
        }
    }
    (dirty, half, icon_key)
}

/// `%<prefix>` of `formats` when SHOW_CHANGESET (p10k:3768-3772).
fn changeset_prefix(o: &Opts, revision: &str) -> String {
    if o.show_changeset {
        format!("{}{} ", o.icons.commit, truncate_bytes(revision, o.hash_len))
    } else {
        String::new()
    }
}

// ---- hg ----

#[derive(Debug, Default)]
struct HgFacts {
    /// `vcs` name after `VCS_INFO_adjust`: `hg`, `hg-git`, ...
    vcs: String,
    csetid: String,
    lrev: String,
    changes: bool,
    branch: String,
    action: String,
    /// `hgmqstring`, empty when the repo has no `.hg/patches`.
    mq: String,
    bookmarks: Vec<String>,
}

/// `HGPLAIN=1 hg id -i -n | read -r r_csetid r_lrev`.
fn parse_hg_id(out: &str) -> (String, String) {
    let line = out.lines().next().unwrap_or("").trim_start();
    match line.split_once(char::is_whitespace) {
        Some((c, l)) => (c.to_string(), l.trim().to_string()),
        None => (line.to_string(), String::new()),
    }
}

/// get_data_hg: a trailing `+` on the local revision marks uncommitted
/// changes; it is stripped from both fields.
fn split_hg_changes(csetid: &str, lrev: &str) -> (String, String, bool) {
    if let Some(l) = lrev.strip_suffix('+') {
        (csetid.strip_suffix('+').unwrap_or(csetid).to_string(), l.to_string(), true)
    } else {
        (csetid.to_string(), lrev.to_string(), false)
    }
}

/// Names in `.hg/bookmarks` (`<hash> <name>` lines) whose hash starts with
/// the working directory's changeset id.
fn parse_hg_bookmarks(file: &str, csetid: &str) -> Vec<String> {
    if csetid.is_empty() {
        return Vec::new();
    }
    file.lines()
        .filter_map(|l| {
            let (hash, name) = l.trim_start().split_once(char::is_whitespace)?;
            let name = name.trim();
            (hash.starts_with(csetid) && !name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

/// Applied mq patches, top of the stack first. Status lines are
/// `<hex>:<name>`; `(Oa)` reverses the order.
fn parse_mq_status(status: &str) -> Vec<String> {
    let mut v: Vec<String> = status
        .lines()
        .map(|l| {
            let hex = l.bytes().take_while(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')).count();
            match l.get(hex..).and_then(|r| r.strip_prefix(':')) {
                Some(name) if hex > 0 => name.to_string(),
                _ => l.to_string(),
            }
        })
        .collect();
    v.reverse();
    v
}

/// `patch-format` `%p (%n applied)` / `nopatch-format` `no patch applied`
/// (VCS_INFO_set-patch-format defaults). `%` in the name is doubled.
fn mq_string(applied: &[String]) -> String {
    match applied.first() {
        Some(top) => format!("{} ({} applied)", top.replace('%', "%%"), applied.len()),
        None => "no patch applied".to_string(),
    }
}

fn hg_message(f: &HgFacts, o: &Opts) -> Message {
    let revision = [f.csetid.as_str(), f.lrev.as_str()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(":");
    let branch = if o.hide_branch_icon {
        f.branch.clone()
    } else {
        format!("{}{}", o.icons.branch, f.branch)
    };
    let mut staged = String::new();
    let mut unstaged = if f.changes { format!(" {}", o.icons.unstaged) } else { String::new() };
    let (dirty, half_dirty, icon_key) =
        run_hooks(&f.vcs, o, &mut staged, &mut unstaged, &SvnFlags::default());
    // `hg-bookmarks` hook: with bookmarks present the hook's string wins.
    let bookmarks = if f.bookmarks.is_empty() {
        String::new()
    } else {
        format!(" {}{}", o.icons.bookmark, f.bookmarks.join(" "))
    };
    let misc = [f.mq.as_str(), bookmarks.as_str()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(";");
    let text = if f.action.is_empty() {
        format!("{}{branch}{staged}{unstaged}{misc}", changeset_prefix(o, &revision))
    } else {
        format!("{branch} %F{{{}}}| {}%f", o.action_fg, f.action)
    };
    Message { text, dirty, half_dirty, icon_key }
}

/// Collect the hg facts for `base` (the dir holding `.hg`), running
/// `hg id -i -n` in `cwd` (VCS_INFO_get_data_hg).
fn hg_facts(bin: &Path, base: &Path, cwd: &Path) -> HgFacts {
    let hgdir = base.join(".hg");
    let read = |name: &str| std::fs::read_to_string(hgdir.join(name)).ok();

    let (csetid, lrev) = run_tool(bin, &["id", "-i", "-n"], cwd, &[("HGPLAIN", "1")])
        .map(|o| parse_hg_id(&o.stdout))
        .unwrap_or_default();
    let (csetid, lrev, changes) = split_hg_changes(&csetid, &lrev);

    let mut branch = read("branch")
        .map(|b| b.trim_end_matches('\n').to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "default".to_string());
    if let Some(topic) = read("topic").and_then(|t| t.lines().next().map(str::to_string)) {
        if !topic.is_empty() {
            branch = format!("{branch}:{topic}");
        }
    }

    let action = if hgdir.join("rebasestate").exists() {
        "rebasing"
    } else if hgdir.join("merge").is_dir() {
        "merging"
    } else {
        ""
    };

    let mq = if hgdir.join("patches").is_dir() {
        let applied = read("patches/status").map(|s| parse_mq_status(&s)).unwrap_or_default();
        mq_string(&applied)
    } else {
        String::new()
    };

    let bookmarks = read("bookmarks")
        .map(|b| parse_hg_bookmarks(&b, &csetid))
        .unwrap_or_default();

    // VCS_INFO_detect_hg flavours -> `vcs` name (overwrite_name).
    let vcs = if hgdir.join("svn").is_dir() {
        "hg-hgsubversion"
    } else if base.join(".hgsvn").is_dir() {
        "hg-hgsvn"
    } else if hgdir.join("git-mapfile").exists() {
        "hg-git"
    } else {
        "hg"
    };

    HgFacts {
        vcs: vcs.to_string(),
        csetid,
        lrev,
        changes,
        branch,
        action: action.to_string(),
        mq,
        bookmarks,
    }
}

// ---- svn ----

/// `svn info` lines into a map: `IFS=: read a b; svninfo[${a// /_}]=${b## #}`.
fn parse_svn_info(out: &str) -> HashMap<String, String> {
    out.lines()
        .map(|l| match l.split_once(':') {
            Some((a, b)) => (a.replace(' ', "_"), b.trim_start_matches(' ').to_string()),
            None => (l.replace(' ', "_"), String::new()),
        })
        .collect()
}

/// `+vi-svn-detect-changes`: `grep ^?`, `^M`, `^A` over `svn status`.
fn parse_svn_status(out: &str) -> SvnFlags {
    SvnFlags {
        untracked: out.lines().any(|l| l.starts_with('?')),
        modified: out.lines().any(|l| l.starts_with('M')),
        added: out.lines().any(|l| l.starts_with('A')),
    }
}

/// Revision shown for an `svn info` failure: `?` for the two errors
/// get_data_svn special-cases (E155036 upgrade required, E155021
/// unsupported format), `None` for any other failure.
fn svn_error_revision(code: Option<i32>, output: &str) -> Option<String> {
    let special = output
        .lines()
        .any(|l| l.starts_with("svn: E155036: ") || l.starts_with("svn: E155021: "));
    (code == Some(1) && special).then(|| "?".to_string())
}

fn svn_message(revision: &str, flags: &SvnFlags, o: &Opts) -> Message {
    let mut staged = String::new();
    let mut unstaged = String::new();
    let (dirty, half_dirty, icon_key) = run_hooks("svn", o, &mut staged, &mut unstaged, flags);
    let text = format!("{}{staged}{unstaged}", changeset_prefix(o, revision));
    Message { text, dirty, half_dirty, icon_key }
}

// ---------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------

/// `vcs_info` over `backends` (git already excluded by the caller), first
/// detected backend wins (vcs_info:105-114). Empty = nothing to show.
pub(crate) fn vcs_info_segments(backends: &[String]) -> Vec<Segment> {
    let start = physical_cwd();
    for backend in backends {
        let (msg, hooks_name) = match backend.as_str() {
            "hg" => {
                let Some(bin) = cmd_on_path("hg") else { continue };
                let Some(base) = bydir_detect(&start, ".hg", &["store", "data", "sharedpath"]) else {
                    continue;
                };
                (VcsRun::Hg(bin, base), "VCS_HG_HOOKS")
            }
            "svn" => {
                let Some(bin) = cmd_on_path("svn") else { continue };
                if bydir_detect(&start, ".svn", &["entries", "format", "wc.db"]).is_none() {
                    continue;
                }
                (VcsRun::Svn(bin), "VCS_SVN_HOOKS")
            }
            other => {
                tracing::debug!(target: "p10k", backend = other, "vcs backend has no native implementation");
                continue;
            }
        };
        let icons = Icons::load();
        let hooks = match hooks_name {
            "VCS_HG_HOOKS" => hook_list(hooks_name, &["vcs-detect-changes"]),
            _ => hook_list(hooks_name, &["vcs-detect-changes", "svn-detect-changes"]),
        };
        let action_fg = p9k_global("VCS_ACTIONFORMAT_FOREGROUND", "1");
        let opts = Opts {
            icons: &icons,
            hooks: &hooks,
            show_changeset: global_bool("SHOW_CHANGESET", false),
            hash_len: global_int("CHANGESET_HASH_LENGTH", 8).max(0) as usize,
            hide_branch_icon: global_bool("HIDE_BRANCH_ICON", false),
            action_fg: &action_fg,
        };
        let message = match msg {
            VcsRun::Hg(bin, base) => Some(hg_message(&hg_facts(&bin, &base, &start), &opts)),
            VcsRun::Svn(bin) => svn_run(&bin, &start, &opts),
        };
        return message.map(build_segment).unwrap_or_default();
    }
    Vec::new()
}

enum VcsRun {
    Hg(PathBuf, PathBuf),
    Svn(PathBuf),
}

/// get_data_svn: `svn info --non-interactive` in the cwd, plus
/// `svn status` when `svn-detect-changes` is enabled.
fn svn_run(bin: &Path, cwd: &Path, o: &Opts) -> Option<Message> {
    let info = run_tool(bin, &["info", "--non-interactive"], cwd, &[])?;
    let revision = if info.code == Some(0) {
        parse_svn_info(&format!("{}{}", info.stdout, info.stderr))
            .remove("Revision")
            .unwrap_or_default()
    } else {
        svn_error_revision(info.code, &format!("{}{}", info.stdout, info.stderr))?
    };
    let flags = if o.hooks.iter().any(|h| h == "svn-detect-changes") {
        run_tool(bin, &["status"], cwd, &[])
            .map(|s| parse_svn_status(&s.stdout))
            .unwrap_or_default()
    } else {
        SvnFlags::default()
    };
    Some(svn_message(&revision, &flags, o))
}

/// `prompt_vcs` tail (p10k:4193-4207): state from the dirty flags, segment
/// only when the message is non-empty.
fn build_segment(msg: Message) -> Vec<Segment> {
    if msg.text.is_empty() {
        return Vec::new();
    }
    let state = if msg.dirty {
        "MODIFIED"
    } else if msg.half_dirty {
        "UNTRACKED"
    } else {
        "CLEAN"
    };
    let icon = msg
        .icon_key
        .and_then(|k| apply_visual_identifier("vcs", Some(state), seg_icon("vcs", Some(state), k)));
    vec![Segment {
        name: "vcs".to_string(),
        state: Some(state.to_string()),
        content: msg.text,
        icon,
        fg: p9k_param("vcs", Some(state), "FOREGROUND", color1()),
        bg: p9k_param("vcs", Some(state), "BACKGROUND", vcs_state_default_bg(state)),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icons() -> Icons {
        Icons {
            branch: "B:".into(),
            commit: "C:".into(),
            staged: "S".into(),
            unstaged: "U".into(),
            untracked: "T".into(),
            bookmark: "K:".into(),
        }
    }

    fn opts<'a>(i: &'a Icons, hooks: &'a [String], show_changeset: bool) -> Opts<'a> {
        Opts {
            icons: i,
            hooks,
            show_changeset,
            hash_len: 8,
            hide_branch_icon: false,
            action_fg: "1",
        }
    }

    fn hooks(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn hg_id_output_parses_and_strips_dirty_marker() {
        let (c, l) = parse_hg_id("5f3a9c1e7b20+ 142+\n");
        assert_eq!((c.as_str(), l.as_str()), ("5f3a9c1e7b20+", "142+"));
        assert_eq!(
            split_hg_changes(&c, &l),
            ("5f3a9c1e7b20".to_string(), "142".to_string(), true)
        );
        let (c, l) = parse_hg_id("5f3a9c1e7b20 142\n");
        assert_eq!(
            split_hg_changes(&c, &l),
            ("5f3a9c1e7b20".to_string(), "142".to_string(), false)
        );
        assert_eq!(parse_hg_id(""), (String::new(), String::new()));
    }

    #[test]
    fn hg_bookmarks_match_by_hash_prefix() {
        let file = "5f3a9c1e7b20aaaaaaaaaaaaaaaaaaaaaaaaaaaa feature-x\n\
                    0000000000000000000000000000000000000000 other\n\
                    5f3a9c1e7b20bbbbbbbbbbbbbbbbbbbbbbbbbbbb release 1\n";
        assert_eq!(
            parse_hg_bookmarks(file, "5f3a9c1e7b20"),
            vec!["feature-x".to_string(), "release 1".to_string()]
        );
        assert!(parse_hg_bookmarks(file, "").is_empty());
    }

    #[test]
    fn mq_status_is_reversed_and_hash_stripped() {
        let status = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2:first.patch\n\
                      0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f:second.patch\n";
        let applied = parse_mq_status(status);
        assert_eq!(applied, vec!["second.patch".to_string(), "first.patch".to_string()]);
        assert_eq!(mq_string(&applied), "second.patch (2 applied)");
        assert_eq!(mq_string(&[]), "no patch applied");
        assert_eq!(mq_string(&["50%.patch".to_string()]), "50%%.patch (1 applied)");
    }

    #[test]
    fn hg_clean_and_dirty_messages() {
        let i = icons();
        let h = hooks(&["vcs-detect-changes"]);
        let mut f = HgFacts {
            vcs: "hg".into(),
            csetid: "5f3a9c1e7b20".into(),
            lrev: "142".into(),
            branch: "default".into(),
            ..Default::default()
        };
        let m = hg_message(&f, &opts(&i, &h, false));
        assert_eq!(
            m,
            Message {
                text: "B:default".into(),
                dirty: false,
                half_dirty: false,
                icon_key: Some("VCS_HG_ICON")
            }
        );
        f.changes = true;
        let m = hg_message(&f, &opts(&i, &h, false));
        assert_eq!(m.text, "B:default U");
        assert!(m.dirty);
    }

    #[test]
    fn hg_changeset_prefix_bookmarks_mq_and_action() {
        let i = icons();
        let h = hooks(&["vcs-detect-changes"]);
        let mut f = HgFacts {
            vcs: "hg".into(),
            csetid: "5f3a9c1e7b20".into(),
            lrev: "142".into(),
            branch: "stable".into(),
            mq: "p.patch (1 applied)".into(),
            bookmarks: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let m = hg_message(&f, &opts(&i, &h, true));
        // %0.8i truncates "5f3a9c1e7b20:142" to 8 bytes.
        assert_eq!(m.text, "C:5f3a9c1e B:stablep.patch (1 applied); K:a b");
        f.action = "merging".into();
        let m = hg_message(&f, &opts(&i, &h, true));
        assert_eq!(m.text, "B:stable %F{1}| merging%f");
    }

    #[test]
    fn hg_flavour_loses_visual_identifier_and_no_hook_means_clean() {
        let i = icons();
        let f = HgFacts {
            vcs: "hg-git".into(),
            branch: "default".into(),
            changes: true,
            ..Default::default()
        };
        let h = hooks(&["vcs-detect-changes"]);
        assert_eq!(hg_message(&f, &opts(&i, &h, false)).icon_key, None);
        let none: Vec<String> = Vec::new();
        let m = hg_message(&HgFacts { vcs: "hg".into(), ..f }, &opts(&i, &none, false));
        assert!(!m.dirty);
        assert_eq!(m.icon_key, None);
    }

    const SVN_INFO: &str = "Path: .\n\
        Working Copy Root Path: /home/u/wc\n\
        URL: https://svn.example.com/repos/proj/trunk\n\
        Relative URL: ^/trunk\n\
        Repository Root: https://svn.example.com/repos/proj\n\
        Revision: 4711\n\
        Node Kind: directory\n\
        Last Changed Date: 2024-05-01 10:11:12 +0000 (Wed, 01 May 2024)\n";

    #[test]
    fn svn_info_keys_and_colon_values() {
        let m = parse_svn_info(SVN_INFO);
        assert_eq!(m["Revision"], "4711");
        assert_eq!(m["URL"], "https://svn.example.com/repos/proj/trunk");
        assert_eq!(m["Working_Copy_Root_Path"], "/home/u/wc");
    }

    #[test]
    fn svn_status_flags_use_first_column_only() {
        let out = "?       scratch.txt\nM       src/a.c\n D      gone.c\nA       src/b.c\n";
        assert_eq!(
            parse_svn_status(out),
            SvnFlags { untracked: true, modified: true, added: true }
        );
        assert_eq!(parse_svn_status(" M      propchange\n"), SvnFlags::default());
        assert_eq!(parse_svn_status(""), SvnFlags::default());
    }

    #[test]
    fn svn_error_codes() {
        let up = "svn: E155036: Please see the 'svn upgrade' command\n";
        assert_eq!(svn_error_revision(Some(1), up), Some("?".to_string()));
        assert_eq!(svn_error_revision(Some(2), up), None);
        assert_eq!(svn_error_revision(Some(1), "svn: E155007: not a working copy\n"), None);
    }

    #[test]
    fn svn_messages_follow_hooks_and_state() {
        let i = icons();
        let h = hooks(&["vcs-detect-changes", "svn-detect-changes"]);
        // Clean tree, no changeset prefix: empty message, segment hidden.
        let clean = svn_message("4711", &SvnFlags::default(), &opts(&i, &h, false));
        assert_eq!(clean.text, "");
        assert!(build_segment(clean).is_empty());
        // Untracked only: UNTRACKED (half dirty), unstaged gets the icon.
        let m = svn_message(
            "4711",
            &SvnFlags { untracked: true, ..Default::default() },
            &opts(&i, &h, false),
        );
        assert_eq!((m.text.as_str(), m.dirty, m.half_dirty), (" T", false, true));
        // Everything: staged first, then unstaged (untracked before modified).
        let m = svn_message(
            "4711",
            &SvnFlags { untracked: true, modified: true, added: true },
            &opts(&i, &h, true),
        );
        assert_eq!(m.text, "C:4711  S T U");
        assert!(m.dirty);
        assert_eq!(m.icon_key, Some("VCS_SVN_ICON"));
    }

    #[test]
    fn revision_truncation_respects_char_boundaries() {
        assert_eq!(truncate_bytes("abcdef", 3), "abc");
        assert_eq!(truncate_bytes("abc", 8), "abc");
        assert_eq!(truncate_bytes("aé", 2), "a");
    }
}
