//! p10k `vcs` segment — every backend except git.
//!
//! Upstream (`~/.zinit/plugins/romkatv---powerlevel10k/internal/p10k.zsh`)
//! renders git through gitstatus and hands every other backend in
//! `POWERLEVEL9K_VCS_BACKENDS` to zsh's `vcs_info` (`prompt_vcs`,
//! p10k:4176-4208) configured by `_p9k_vcs_info_init` (p10k:3765-3808) and
//! the `+vi-*` hooks (p10k:3681-3763). The engine does not load
//! `vcs_info`; this module is the equivalent logic, ported from the zsh
//! sources it replaces (`Functions/VCS_Info/`): `vcs_info` (the driver in
//! [`vcs_info`]), `VCS_INFO_formats` ([`Run::formats`]),
//! `VCS_INFO_set-branch-format`, `VCS_INFO_set-patch-format`,
//! `VCS_INFO_reposub`, `VCS_INFO_bydir_detect`, `VCS_INFO_nvcsformats`,
//! and `Backends/VCS_INFO_{detect,get_data}_{hg,svn}` here;
//! bzr/cdv/cvs/darcs/fossil/mtn/p4/svk/tla live in `vcs_backends.rs`;
//! the hook runner (`VCS_INFO_hook`) is `vcs_hooks.rs`.
//!
//! Effective `vcs_info` configuration (`_p9k_vcs_info_init`), installed into
//! the global zstyle table by [`theme_styles`] / `sync_global_styles` the
//! way the theme's own `zstyle` calls land: `setstypat` replaces an
//! identical pattern and otherwise ranks by specificity, so a user's more
//! specific pattern beats the theme's and a less specific one loses:
//! - `formats`          `<prefix>%b%c%u%m` (svn: `<prefix>%c%u`);
//!   `<prefix>` = `VCS_COMMIT_ICON%0.<CHANGESET_HASH_LENGTH>i ` when
//!   `SHOW_CHANGESET`.
//! - `actionformats`    `%b %F{VCS_ACTIONFORMAT_FOREGROUND}| %a%f`
//!   (svn: `<prefix>%c%u %F{...}| %a%f`)
//! - `stagedstr` / `unstagedstr`  ` <VCS_STAGED_ICON>` / ` <VCS_UNSTAGED_ICON>`
//! - hg `branchformat`  `<VCS_BRANCH_ICON>%b` (`%b` with HIDE_BRANCH_ICON),
//!   `get-revision` and `get-bookmarks` on; everywhere check-for-changes on.
//! - hooks              `set-message` for `hg*` / `svn*` from
//!   `VCS_HG_HOOKS` / `VCS_SVN_HOOKS`, `gen-hg-bookmark-string` for `hg*`
//!   is `hg-bookmarks`. The theme's own hooks `vcs-detect-changes`,
//!   `svn-detect-changes` and `hg-bookmarks` run natively; any other hook
//!   name is the user's `+vi-NAME` shell function (see `vcs_hooks.rs`).
//!
//! Segment state follows `prompt_vcs`: `VCS_WORKDIR_DIRTY` -> MODIFIED,
//! else `VCS_WORKDIR_HALF_DIRTY` -> UNTRACKED, else CLEAN; an empty
//! message (e.g. a clean svn tree) hides the segment.
//!
//! The quilt add-on and standalone modes (`VCS_INFO_quilt`) are in
//! `vcs_quilt.rs`; they stay off unless the user enables the `use-quilt`
//! style. `max-exports` bounds the messages like `VCS_INFO_formats` does,
//! and every message is exported as `vcs_info_msg_<N>_` (`VCS_INFO_set`),
//! though the segment shows `vcs_info_msg_0_`.

use crate::extensions::p10k::config::{p9k_global, p9k_param};
use crate::extensions::p10k::render::Segment;
use crate::extensions::p10k::segments_core::vcs_state_default_bg;
use crate::extensions::p10k::segments_sys::cmd_on_path;
use crate::extensions::p10k::shared::{
    apply_visual_identifier, color1, global_bool, global_int, seg_icon,
};
use crate::extensions::p10k::vcs_hooks::{
    any_matches_joined, call_shell_function, read_shell_param, run_hook, style_context,
    subst_pattern_matches, zsh_tail, Assoc, HookHost, HookState, ParamValue, Specs,
};
use crate::ported::modules::zutil::{lookupstyle, style_table, zstyletab};
use crate::ported::params::{getaparam, getsparam, setsparam};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Hard latency budget per tool invocation; the child is killed on overrun.
const TOOL_BUDGET: Duration = Duration::from_secs(5);

/// The backends `vcs_info` ships (`Backends/VCS_INFO_get_data_*`), i.e. the
/// valid `VCS_BACKENDS` values; `git` is rendered by the caller.
pub(crate) const BACKENDS: [&str; 12] = [
    "bzr", "cdv", "cvs", "darcs", "fossil", "git", "hg", "mtn", "p4", "svk", "svn", "tla",
];

// ---------------------------------------------------------------------
// Subprocess
// ---------------------------------------------------------------------

pub(crate) struct ToolOut {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Run `bin args` in `dir` with extra environment, capturing stdout and
/// stderr separately. `None` on spawn failure or budget overrun.
///
/// `vcs_info` runs every tool with `LC_MESSAGES=C` and, when `LC_ALL` is
/// set, `LANG=$LC_ALL` with `LC_ALL` cleared (vcs_info:55-60), so the
/// parsed output is locale independent.
pub(crate) fn run_tool(
    bin: &Path,
    args: &[&str],
    dir: &Path,
    env: &[(&str, &str)],
) -> Option<ToolOut> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_MESSAGES", "C");
    if let Ok(lc_all) = std::env::var("LC_ALL") {
        if !lc_all.is_empty() {
            cmd.env("LANG", lc_all).env_remove("LC_ALL");
        }
    }
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

/// `VCS_INFO_check_com`: an absolute path must be executable, anything
/// else must be a command on `$PATH`. Returns the resolved binary.
pub(crate) fn tool_bin(cmd: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if cmd.starts_with('/') {
        let ok = std::fs::metadata(cmd)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
        ok.then(|| PathBuf::from(cmd))
    } else {
        cmd_on_path(cmd)
    }
}

// ---------------------------------------------------------------------
// Detection — VCS_INFO_bydir_detect
// ---------------------------------------------------------------------

/// Walk from `start` towards `/` (exclusive) for a directory holding
/// `dirname/` that contains at least one of `need` — the
/// `vcs_comm[detect_need_file]` form of `VCS_INFO_bydir_detect`; an empty
/// `need` accepts any such directory (no `detect_need_file`). An
/// unreadable ancestor aborts the walk. Returns the repo base directory.
pub(crate) fn bydir_detect(start: &Path, dirname: &str, need: &[&str]) -> Option<PathBuf> {
    let mut base = start.to_path_buf();
    while base != Path::new("/") {
        std::fs::read_dir(&base).ok()?; // `[[ -r ${basedir} ]] || return 1`
        let marker = base.join(dirname);
        if marker.is_dir() && (need.is_empty() || need.iter().any(|f| marker.join(f).exists())) {
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
// Configuration the theme layers over vcs_info (_p9k_vcs_info_init)
// ---------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct Icons {
    pub branch: String,
    pub commit: String,
    pub staged: String,
    pub unstaged: String,
    pub untracked: String,
    pub bookmark: String,
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

/// Snapshot of the `POWERLEVEL9K_*` values `_p9k_vcs_info_init` reads.
#[derive(Clone)]
pub(crate) struct P10kCfg {
    pub icons: Icons,
    pub show_changeset: bool,
    pub hash_len: usize,
    pub hide_branch_icon: bool,
    pub action_fg: String,
    pub git_hooks: Vec<String>,
    pub hg_hooks: Vec<String>,
    pub svn_hooks: Vec<String>,
}

/// `POWERLEVEL9K_<name>` as an array with a declared default (p10k:7744).
/// A set-but-empty array disables every hook, so it must not fall back.
fn hook_list(name: &str, default: &[&str]) -> Vec<String> {
    let full = format!("POWERLEVEL9K_{name}");
    getaparam(&full)
        .or_else(|| getsparam(&full).map(|s| vec![s]))
        .unwrap_or_else(|| default.iter().map(|s| s.to_string()).collect())
}

impl P10kCfg {
    fn load() -> Self {
        P10kCfg {
            icons: Icons::load(),
            show_changeset: global_bool("SHOW_CHANGESET", false),
            hash_len: global_int("CHANGESET_HASH_LENGTH", 8).max(0) as usize,
            hide_branch_icon: global_bool("HIDE_BRANCH_ICON", false),
            action_fg: p9k_global("VCS_ACTIONFORMAT_FOREGROUND", "1"),
            git_hooks: hook_list(
                "VCS_GIT_HOOKS",
                &[
                    "vcs-detect-changes",
                    "git-untracked",
                    "git-aheadbehind",
                    "git-stash",
                    "git-remotebranch",
                    "git-tagname",
                ],
            ),
            hg_hooks: hook_list("VCS_HG_HOOKS", &["vcs-detect-changes"]),
            svn_hooks: hook_list("VCS_SVN_HOOKS", &["vcs-detect-changes", "svn-detect-changes"]),
        }
    }

    /// `<prefix>` of `formats` (p10k:3768-3772).
    fn prefix(&self) -> String {
        if self.show_changeset {
            format!("{}%0.{}i ", self.icons.commit, self.hash_len)
        } else {
            String::new()
        }
    }
}

/// One `zstyle <pattern> <style> <values...>` call of the theme.
pub(crate) type ThemeStyle = (&'static str, &'static str, Vec<String>);

/// The `zstyle` calls of `_p9k_vcs_info_init` (p10k:3774-3807), in order.
pub(crate) fn theme_styles(cfg: &P10kCfg) -> Vec<ThemeStyle> {
    let one = |s: String| vec![s];
    let fg = &cfg.action_fg;
    let prefix = cfg.prefix();
    let branchformat = if cfg.hide_branch_icon {
        "%b".to_string()
    } else {
        format!("{}%b", cfg.icons.branch)
    };
    vec![
        (":vcs_info:*", "check-for-changes", one("true".into())),
        (":vcs_info:*", "formats", one(format!("{prefix}%b%c%u%m"))),
        (":vcs_info:*", "actionformats", one(format!("%b %F{{{fg}}}| %a%f"))),
        (":vcs_info:*", "stagedstr", one(format!(" {}", cfg.icons.staged))),
        (":vcs_info:*", "unstagedstr", one(format!(" {}", cfg.icons.unstaged))),
        (":vcs_info:git*+set-message:*", "hooks", cfg.git_hooks.clone()),
        (":vcs_info:hg*+set-message:*", "hooks", cfg.hg_hooks.clone()),
        (":vcs_info:svn*+set-message:*", "hooks", cfg.svn_hooks.clone()),
        (":vcs_info:hg*:*", "branchformat", one(branchformat)),
        (":vcs_info:hg*:*", "get-revision", one("true".into())),
        (":vcs_info:hg*:*", "get-bookmarks", one("true".into())),
        (":vcs_info:hg*+gen-hg-bookmark-string:*", "hooks", one("hg-bookmarks".into())),
        (":vcs_info:svn*:*", "formats", one(format!("{prefix}%c%u"))),
        (
            ":vcs_info:svn*:*",
            "actionformats",
            one(format!("{prefix}%c%u %F{{{fg}}}| %a%f")),
        ),
        (":vcs_info:*", "get-revision", one(cfg.show_changeset.to_string())),
    ]
}

/// `zstyle` each of `styles` into `table`. `setstypat` replaces an
/// identical pattern and otherwise ranks by specificity, exactly as for
/// a user's own `zstyle` calls.
pub(crate) fn install_theme_styles(table: &mut style_table, styles: &[ThemeStyle]) {
    for (pattern, style, values) in styles {
        table.set(pattern, style, values.clone(), None);
    }
}

/// The styles last installed into the global table.
static INSTALLED_STYLES: Mutex<Option<Vec<ThemeStyle>>> = Mutex::new(None);

/// `_p9k_vcs_info_init` + the per-prompt `zstyle ':vcs_info:*' enable
/// ${backends}` (p10k:4185) against the global `zstyle` table. The init
/// calls repeat only when the configuration they derive from changes (what
/// `p10k reload` does), so a `zstyle` the user runs afterwards is not
/// overwritten on every prompt.
fn sync_global_styles(cfg: &P10kCfg, backends: &[String]) {
    let styles = theme_styles(cfg);
    let Ok(mut installed) = INSTALLED_STYLES.lock() else { return };
    let Ok(mut table) = zstyletab.lock() else { return };
    if installed.as_ref() != Some(&styles) {
        install_theme_styles(&mut table, &styles);
        *installed = Some(styles);
    }
    table.set(":vcs_info:*", "enable", backends.to_vec(), None);
}

// ---------------------------------------------------------------------
// The host: zstyle table + theme defaults, native hooks, user functions
// ---------------------------------------------------------------------

pub(crate) struct PromptHost {
    pub cfg: P10kCfg,
    pub cwd: PathBuf,
    /// A private zstyle table, for tests; `None` reads the global one.
    pub table: Option<style_table>,
    /// Canned `svn status` output, for tests; `None` runs the real tool.
    pub svn_status: Option<String>,
}

/// `svn status` first-column classes consumed by `+vi-svn-detect-changes`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct SvnFlags {
    untracked: bool,
    modified: bool,
    added: bool,
}

/// `+vi-svn-detect-changes`: `grep ^?`, `^M`, `^A` over `svn status`.
fn parse_svn_status(out: &str) -> SvnFlags {
    SvnFlags {
        untracked: out.lines().any(|l| l.starts_with('?')),
        modified: out.lines().any(|l| l.starts_with('M')),
        added: out.lines().any(|l| l.starts_with('A')),
    }
}

impl PromptHost {
    fn svn_status_output(&self) -> String {
        if let Some(canned) = &self.svn_status {
            return canned.clone();
        }
        cmd_on_path("svn")
            .and_then(|bin| run_tool(&bin, &["status"], &self.cwd, &[]))
            .map(|o| o.stdout)
            .unwrap_or_default()
    }
}

impl HookHost for PromptHost {
    /// `zstyle -a`: the theme's styles and the user's share one table.
    fn style(&self, ctx: &str, style: &str) -> Vec<String> {
        match &self.table {
            Some(t) => t.get_match(ctx, style).map(|(vals, _)| vals).unwrap_or_default(),
            None => lookupstyle(ctx, style),
        }
    }

    /// p10k:3716-3748 — the theme's own `+vi-*` functions.
    fn native_hook(&self, name: &str, _args: &[String], st: &mut HookState) -> Option<i32> {
        let icons = &self.cfg.icons;
        match name {
            "vcs-detect-changes" => {
                match st.hook_com.get("vcs") {
                    "hg" => st.flags.visual_identifier = "VCS_HG_ICON".into(),
                    "svn" => st.flags.visual_identifier = "VCS_SVN_ICON".into(),
                    _ => {}
                }
                st.flags.dirty =
                    !st.hook_com.get("staged").is_empty() || !st.hook_com.get("unstaged").is_empty();
                Some(0)
            }
            "svn-detect-changes" => {
                let flags = parse_svn_status(&self.svn_status_output());
                if flags.untracked {
                    st.hook_com.append("unstaged", &format!(" {}", icons.untracked));
                    st.flags.half_dirty = true;
                }
                if flags.modified {
                    st.hook_com.append("unstaged", &format!(" {}", icons.unstaged));
                    st.flags.dirty = true;
                }
                if flags.added {
                    st.hook_com.append("staged", &format!(" {}", icons.staged));
                    st.flags.dirty = true;
                }
                Some(0)
            }
            "hg-bookmarks" => {
                let marks = st.array("hgbmarks");
                if !marks.is_empty() {
                    let s = format!(" {}{}", icons.bookmark, marks.join(" "));
                    st.hook_com.set("hg-bookmark-string", s);
                    st.ret = 1;
                }
                Some(0)
            }
            _ => None,
        }
    }

    fn user_function_exists(&self, func: &str) -> bool {
        crate::ported::utils::getshfunc(func).is_some()
    }

    fn call_user_function(&self, func: &str, args: &[String], st: &mut HookState) -> i32 {
        call_shell_function(func, args, st)
    }

    fn param(&self, name: &str) -> ParamValue {
        read_shell_param(name, true)
    }
}

// ---------------------------------------------------------------------
// vcs_info state + the shared functions (VCS_INFO_formats & friends)
// ---------------------------------------------------------------------

/// What `vcs_info` leaves behind for `prompt_vcs`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Message {
    pub text: String,
    pub dirty: bool,
    pub half_dirty: bool,
    /// `vcs_visual_identifier`: an icon key such as `VCS_HG_ICON`.
    pub icon_key: Option<String>,
    /// What `VCS_INFO_set` writes to the `vcs_info_msg_<N>_` globals.
    pub exports: Exports,
}

/// The state `VCS_INFO_set` publishes as `vcs_info_msg_<N>_`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Exports {
    /// `msgs`, index `N` being `vcs_info_msg_<N>_`.
    pub msgs: Vec<String>,
    /// `maxexports` at the time.
    pub maxexports: usize,
    /// `VCS_INFO_set --nvcs`: every message is cleared first.
    pub nvcs: bool,
    /// `vcs_info` returned without calling `VCS_INFO_set`.
    pub kept: bool,
}

/// One `vcs_info` invocation.
pub(crate) struct Run<'a> {
    pub host: &'a dyn HookHost,
    pub st: HookState,
    /// `${PWD:P}`
    pub cwd: PathBuf,
    /// `$PWD`
    pub pwd: String,
    /// `VCS_INFO_set --nvcs` ran.
    pub nvcs: bool,
}

/// `zformat -f` with single-character specs.
pub(crate) fn zformat(fmt: &str, specs: &[(char, &str)]) -> String {
    let map: Specs = specs.iter().map(|(c, v)| (*c, v.to_string())).collect();
    crate::ported::modules::zutil::zformat_substring(fmt, &map, false)
}

/// `VCS_INFO_reposub`: `base`'s sub-directory of the physical cwd, `.` at
/// (or outside) the repository root.
pub(crate) fn reposub(base: &str, cwd: &Path) -> String {
    let base = base.trim_end_matches('/');
    let tmp = cwd.to_string_lossy();
    match tmp.strip_prefix(&format!("{base}/")) {
        Some(rest) => rest.to_string(),
        None => ".".to_string(),
    }
}

impl<'a> Run<'a> {
    pub(crate) fn new(host: &'a dyn HookHost, cwd: PathBuf, pwd: String) -> Self {
        Run { host, st: HookState::new(), cwd, pwd, nvcs: false }
    }

    fn ctx(&self) -> String {
        style_context(&self.st.vars)
    }

    pub(crate) fn style_a(&self, style: &str) -> Vec<String> {
        self.host.style(&self.ctx(), style)
    }

    pub(crate) fn style_s(&self, style: &str) -> Option<String> {
        self.host.style_s(&self.ctx(), style)
    }

    pub(crate) fn style_t(&self, style: &str) -> bool {
        self.host.style_t(&self.ctx(), style)
    }

    pub(crate) fn style_tt(&self, style: &str) -> bool {
        self.host.style_tt(&self.ctx(), style)
    }

    /// `VCS_INFO_hook`.
    pub(crate) fn hook(&mut self, name: &str, args: &[String]) -> i32 {
        run_hook(self.host, &mut self.st, name, args)
    }

    /// `vcs_comm[cmd]` resolved to a binary.
    pub(crate) fn cmd_bin(&self) -> Option<PathBuf> {
        tool_bin(self.st.vcs_comm.get("cmd"))
    }

    /// Run the backend's command with `args` in `dir`.
    pub(crate) fn tool(&self, args: &[&str], dir: &Path, env: &[(&str, &str)]) -> Option<ToolOut> {
        run_tool(&self.cmd_bin()?, args, dir, env)
    }

    /// `VCS_INFO_get_cmd`.
    fn get_cmd(&mut self) {
        let cmd = self
            .style_s("command")
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| self.st.vars.vcs.clone());
        self.st.vcs_comm.set("cmd", cmd);
    }

    /// `VCS_INFO_maxexports`.
    fn read_maxexports(&mut self) {
        let n = self.style_s("max-exports");
        self.st.vars.maxexports = match n.as_deref() {
            None => 2,
            Some(s) => match s.parse::<usize>() {
                Ok(v) if v >= 1 && s.bytes().all(|b| b.is_ascii_digit()) => v,
                _ => 2,
            },
        };
    }

    /// Run `f` with `init` as `hook_com` (a backend's `local -A hook_com`),
    /// restoring the caller's afterwards.
    pub(crate) fn with_hook_com<R>(&mut self, init: Assoc, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = std::mem::replace(&mut self.st.hook_com, init);
        let r = f(self);
        self.st.hook_com = saved;
        r
    }

    /// `VCS_INFO_adjust`.
    pub(crate) fn adjust(&mut self) {
        let name = self.st.vcs_comm.get("overwrite_name").to_string();
        if !name.is_empty() {
            self.st.vars.vcs = name;
        }
    }

    /// `VCS_INFO_set-branch-format`: `None` when `rrn` is empty (the
    /// function fails), else the formatted branch.
    pub(crate) fn set_branch_format(&mut self, branch: &str, revision: &str) -> Option<String> {
        if self.st.vars.rrn.is_empty() {
            return None;
        }
        let fmt = self.style_s("branchformat").unwrap_or_else(|| "%b:%r".to_string());
        let init = Assoc::from_pairs(&[("branch", branch), ("revision", revision)]);
        Some(self.with_hook_com(init, |r| {
            if r.hook("set-branch-format", &[fmt.clone()]) == 0 {
                let b = r.st.hook_com.get("branch").to_string();
                let rev = r.st.hook_com.get("revision").to_string();
                zformat(&fmt, &[('b', b.as_str()), ('r', rev.as_str())])
            } else {
                r.st.hook_com.get("branch-replace").to_string()
            }
        }))
    }

    /// `VCS_INFO_set-patch-format` for the hg mq patch list. `extra` is
    /// the backend's `extra_hook_com` (`guards`, `guards-n`); its `%g` and
    /// `%G` formats come from `VCS_INFO_hg_extra_zformats`.
    pub(crate) fn set_patch_format(
        &mut self,
        applied: &[String],
        unapplied: &[String],
        extra: &Assoc,
    ) -> String {
        let ctx = self.ctx();
        self.set_patch_format_in(applied, unapplied, &ctx, extra, &Assoc::default(), &|st| {
            vec![
                ('g', st.hook_com.get("guards").to_string()),
                ('G', st.array("mqguards").len().to_string()),
            ]
        })
    }

    /// `VCS_INFO_set-patch-format applied-array applied-string
    /// unapplied-array unapplied-string ctx fmt-var set_extra reply_fn
    /// gen_extra`, with the arrays passed directly. `ctx` is the context
    /// the `patch-format` / `nopatch-format` styles are read from;
    /// `gen_extra` joins `hook_com` for the `gen-*-string` hooks and
    /// `set_extra` for `set-patch-format`; `reply` is the `$8` function —
    /// it runs after the hook and yields the extra `zformat` specs.
    pub(crate) fn set_patch_format_in(
        &mut self,
        applied: &[String],
        unapplied: &[String],
        ctx: &str,
        set_extra: &Assoc,
        gen_extra: &Assoc,
        reply: &dyn Fn(&HookState) -> Vec<(char, String)>,
    ) -> String {
        self.st.hook_com = gen_extra.clone();
        let mut applied_escape = false;
        let applied_string = if self.hook("gen-applied-string", applied) == 0 {
            applied_escape = true;
            applied.first().cloned().unwrap_or_default()
        } else {
            self.st.hook_com.get("applied-string").to_string()
        };
        self.st.hook_com = gen_extra.clone();
        let mut unapplied_escape = false;
        let unapplied_string = if self.hook("gen-unapplied-string", unapplied) == 0 {
            unapplied_escape = true;
            unapplied.len().to_string()
        } else {
            self.st.hook_com.get("unapplied-string").to_string()
        };
        self.st.hook_com.clear();

        let fmt = if applied.is_empty() {
            self.host.style_s(ctx, "nopatch-format").unwrap_or_else(|| "no patch applied".into())
        } else {
            self.host.style_s(ctx, "patch-format").unwrap_or_else(|| "%p (%n applied)".into())
        };
        let (an, un) = (applied.len(), unapplied.len());
        let init = Assoc::from_pairs(&[
            ("applied-n", an.to_string().as_str()),
            ("applied", applied_string.as_str()),
            ("unapplied-n", un.to_string().as_str()),
            ("unapplied", unapplied_string.as_str()),
            ("all-n", (an + un).to_string().as_str()),
        ]);
        self.st.hook_com = init;
        for (k, v) in set_extra.pairs() {
            self.st.hook_com.set(k, v.as_str());
        }
        let out = if self.hook("set-patch-format", &[fmt.clone()]) == 0 {
            if applied_escape {
                let v = self.st.hook_com.get("applied").replace('%', "%%");
                self.st.hook_com.set("applied", v);
            }
            if unapplied_escape {
                let v = self.st.hook_com.get("unapplied").replace('%', "%%");
                self.st.hook_com.set("unapplied", v);
            }
            let hc = &self.st.hook_com;
            let extra_specs = reply(&self.st);
            let mut specs: Vec<(char, &str)> = vec![
                ('p', hc.get("applied")),
                ('u', hc.get("unapplied")),
                ('n', hc.get("applied-n")),
                ('c', hc.get("unapplied-n")),
                ('a', hc.get("all-n")),
            ];
            specs.extend(extra_specs.iter().map(|(c, v)| (*c, v.as_str())));
            zformat(&fmt, &specs)
        } else {
            self.st.hook_com.get("patch-replace").to_string()
        };
        self.st.hook_com.clear();
        out
    }

    /// `VCS_INFO_formats action branch base staged unstaged revision misc`:
    /// fill `msgs` from the (action)formats style after the post-backend
    /// and set-message hooks.
    pub(crate) fn formats(
        &mut self,
        action: &str,
        branch: &str,
        base: &str,
        staged: &str,
        unstaged: &str,
        revision: &str,
        misc: &str,
    ) {
        let vcs = self.st.vars.vcs.clone();
        let base_name = zsh_tail(base).to_string();
        let subdir = reposub(base, &self.cwd);
        self.st.hook_com = Assoc::from_pairs(&[
            ("action", action),
            ("action_orig", action),
            ("branch", branch),
            ("branch_orig", branch),
            ("base", base),
            ("base_orig", base),
            ("staged", staged),
            ("staged_orig", staged),
            ("unstaged", unstaged),
            ("unstaged_orig", unstaged),
            ("revision", revision),
            ("revision_orig", revision),
            ("misc", misc),
            ("misc_orig", misc),
            ("vcs", vcs.as_str()),
            ("vcs_orig", vcs.as_str()),
            ("base-name", base_name.as_str()),
            ("base-name_orig", base_name.as_str()),
            ("subdir", subdir.as_str()),
            ("subdir_orig", subdir.as_str()),
        ]);
        self.hook("post-backend", &[]);

        let (style, fallback) = if self.st.hook_com.get("action").is_empty() {
            ("formats", " (%s)-[%b]%u%c-")
        } else {
            ("actionformats", " (%s)-[%b|%a]%u%c-")
        };
        let mut msgs = self.style_a(style);
        if msgs.is_empty() {
            msgs.push(fallback.to_string());
        }
        for (key, default, style) in [("staged", "S", "stagedstr"), ("unstaged", "U", "unstagedstr")] {
            if !self.st.hook_com.get(key).is_empty() {
                let s = self.style_s(style).filter(|s| !s.is_empty());
                self.st.hook_com.set(key, s.unwrap_or_else(|| default.to_string()));
            }
        }
        // `if quiltmode != standalone && VCS_INFO_hook pre-addon-quilt; then
        // addon; elif quiltmode == standalone; then quilt=misc; fi` — a
        // vetoed add-on leaves `hook_com[quilt]` empty.
        let standalone = self.st.vars.quiltmode == "standalone";
        let quilt = if !standalone && self.hook("pre-addon-quilt", &[]) == 0 {
            self.quilt("addon").1
        } else if standalone {
            self.st.hook_com.get("misc").to_string()
        } else {
            String::new()
        };
        self.st.hook_com.set("quilt", quilt);

        msgs.truncate(self.st.vars.maxexports);
        self.st.msgs = msgs.clone();
        for i in 0..msgs.len() {
            let msg = msgs[i].clone();
            let args = [i.to_string(), msg.clone()];
            let formatted = if self.hook("set-message", &args) == 0 {
                let hc = &self.st.hook_com;
                zformat(
                    &msg,
                    &[
                        ('a', hc.get("action")),
                        ('b', hc.get("branch")),
                        ('c', hc.get("staged")),
                        ('i', hc.get("revision")),
                        ('m', hc.get("misc")),
                        ('r', hc.get("base-name")),
                        ('s', hc.get("vcs")),
                        ('u', hc.get("unstaged")),
                        ('Q', hc.get("quilt")),
                        ('R', hc.get("base")),
                        ('S', hc.get("subdir")),
                    ],
                )
            } else {
                self.st.hook_com.get("message").to_string()
            };
            msgs[i] = formatted;
            self.st.msgs = msgs.clone();
        }
        self.st.msgs = msgs;
        self.st.hook_com.clear();
        self.st.backend_misc.clear();
    }

    /// `VCS_INFO_set --nvcs`: the `nvcsformats` messages and the `no-vcs`
    /// hook (`VCS_INFO_nvcsformats` keeps `maxexports - 1` entries).
    fn set_nvcs(&mut self) {
        self.nvcs = true;
        let mut msgs = self.style_a("nvcsformats");
        let max = self.st.vars.maxexports;
        if msgs.len() > max {
            msgs.truncate(max.saturating_sub(1));
        }
        self.st.msgs = msgs;
        self.hook("no-vcs", &[]);
    }

    /// The message `vcs_info_msg_0_` ends up with.
    fn message(&self) -> Message {
        Message {
            text: self.st.msgs.first().cloned().unwrap_or_default(),
            dirty: self.st.flags.dirty,
            half_dirty: self.st.flags.half_dirty,
            icon_key: Some(self.st.flags.visual_identifier.clone()).filter(|k| !k.is_empty()),
            exports: Exports {
                msgs: self.st.msgs.clone(),
                maxexports: self.st.vars.maxexports,
                nvcs: self.nvcs,
                kept: false,
            },
        }
    }
}

// ---------------------------------------------------------------------
// hg — Backends/VCS_INFO_{detect,get_data}_hg
// ---------------------------------------------------------------------

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

/// Applied mq patches, top of the stack first: status lines are
/// `<hex>:<name>` (`(Oa)` reverses the order); empty lines vanish.
fn parse_mq_status(status: &str) -> Vec<String> {
    let mut v: Vec<String> = status
        .lines()
        .filter(|l| !l.is_empty())
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

/// Active guards from `.hg/patches/guards`, ascending (`(oa)`).
fn parse_mq_guards(file: &str) -> Vec<String> {
    let mut v: Vec<String> = file.lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
    v.sort();
    v
}

/// Patches of the series file that are neither applied nor excluded by
/// guards (get_data_hg, the `get-unapplied` loop).
fn parse_mq_unapplied(series: &str, applied: &[String], active: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for line in series.lines() {
        let line = line.trim_start();
        let (patch, rest) = match line.split_once(char::is_whitespace) {
            Some((p, r)) => (p, r.trim_start()),
            None => (line, ""),
        };
        if patch.is_empty() || patch.starts_with('#') || applied.iter().any(|a| a == patch) {
            continue;
        }
        let guards: Vec<&str> = rest.split(' ').filter(|g| !g.is_empty()).collect();
        let strip = |marker: &str| -> Vec<String> {
            guards
                .iter()
                .filter(|g| g.contains(marker))
                .map(|g| g.strip_prefix(marker).unwrap_or(g).to_string())
                .collect()
        };
        let (neg, pos) = (strip("#-"), strip("#+"));
        if !neg.is_empty() && any_matches_joined(active, &neg) {
            continue;
        }
        if !pos.is_empty() {
            if any_matches_joined(active, &pos) {
                out.push(patch.to_string());
            }
            continue;
        }
        out.push(patch.to_string());
    }
    out
}

/// `VCS_INFO_hexdump file n`: lower-case hex of the first `n` bytes.
fn hexdump(file: &Path, n: usize) -> Option<String> {
    let mut buf = vec![0u8; n];
    let mut f = std::fs::File::open(file).ok()?;
    let mut got = 0;
    while got < n {
        match f.read(&mut buf[got..]).ok()? {
            0 => break,
            k => got += k,
        }
    }
    Some(buf[..got].iter().map(|b| format!("{b:02x}")).collect())
}

/// The mq files of `.hg/patches`.
#[derive(Debug, Default)]
pub(crate) struct MqFiles {
    status: Option<String>,
    series: Option<String>,
    guards: Option<String>,
}

/// Everything get_data_hg reads from the repository and the `hg` tool.
#[derive(Debug, Default)]
pub(crate) struct HgFacts {
    csetid: String,
    lrev: String,
    /// Contents of `.hg/branch`, newlines stripped.
    branch_file: String,
    /// First line of `.hg/topic` when the file is non-empty.
    topic: Option<String>,
    merging: bool,
    rebasing: bool,
    bookmarks_file: Option<String>,
    /// Contents of `.hg/bookmarks.current`, newlines stripped.
    current_bookmark: Option<String>,
    /// `Some` when `.hg/patches` exists.
    mq: Option<MqFiles>,
}

impl Run<'_> {
    pub(crate) fn detect_hg(&mut self) -> bool {
        if tool_bin(self.st.vcs_comm.get("cmd")).is_none() {
            return false;
        }
        let Some(base) = bydir_detect(&self.cwd, ".hg", &["store", "data", "sharedpath"]) else {
            return false;
        };
        let name = if base.join(".hg/svn").is_dir() {
            "hg-hgsubversion"
        } else if base.join(".hgsvn").is_dir() {
            "hg-hgsvn"
        } else if base.join(".hg/git-mapfile").exists() {
            "hg-git"
        } else {
            ""
        };
        self.st.vcs_comm.set("overwrite_name", name);
        self.st.vcs_comm.set("basedir", base.to_string_lossy().into_owned());
        true
    }

    pub(crate) fn get_data_hg(&mut self) -> bool {
        let base = PathBuf::from(self.st.vcs_comm.get("basedir"));
        self.st.vars.rrn = zsh_tail(&base.to_string_lossy()).to_string();
        self.adjust();
        let hgdir = base.join(".hg");
        let read = |name: &str| std::fs::read_to_string(hgdir.join(name)).ok();
        let chomp = |s: String| s.trim_end_matches('\n').to_string();

        let (mut csetid, mut lrev) = (String::new(), String::new());
        if self.style_t("get-revision") {
            let simple = if self.style_t("use-simple") {
                hexdump(&hgdir.join("dirstate"), 20)
            } else {
                None
            };
            if let Some(hex) = simple {
                csetid = hex;
            } else {
                let mut args = vec!["id", "-i", "-n"];
                if !self.style_t("check-for-changes") {
                    args.push("-r.");
                }
                if let Some(o) = self.tool(&args, &self.cwd.clone(), &[("HGPLAIN", "1")]) {
                    (csetid, lrev) = parse_hg_id(&o.stdout);
                }
            }
        }
        let topic = std::fs::metadata(hgdir.join("topic"))
            .ok()
            .filter(|m| m.is_file() && m.len() > 0)
            .and_then(|_| read("topic"))
            .map(|t| t.lines().next().unwrap_or("").to_string());
        let mq = hgdir.join("patches").is_dir().then(|| MqFiles {
            status: read("patches/status"),
            series: read("patches/series"),
            guards: read("patches/guards"),
        });
        let facts = HgFacts {
            csetid,
            lrev,
            branch_file: read("branch").map(chomp).unwrap_or_default(),
            topic,
            merging: hgdir.join("merge").is_dir(),
            rebasing: hgdir.join("rebasestate").exists(),
            bookmarks_file: read("bookmarks"),
            current_bookmark: read("bookmarks.current").map(chomp),
            mq,
        };
        self.hg_finish(&base.to_string_lossy(), facts);
        true
    }

    /// The rest of get_data_hg once the repository has been read.
    pub(crate) fn hg_finish(&mut self, hgbase: &str, f: HgFacts) {
        let (mut csetid, mut lrev) = (f.csetid, f.lrev);
        let mut branch = if f.branch_file.is_empty() { "default".to_string() } else { f.branch_file };
        if let Some(topic) = f.topic {
            branch = format!("{branch}:{topic}");
        }
        let mut changes = "";
        if lrev.ends_with('+') {
            changes = "1";
            lrev.pop();
            if csetid.ends_with('+') {
                csetid.pop();
            }
        }
        let action = if f.rebasing {
            "rebasing"
        } else if f.merging {
            "merging"
        } else {
            ""
        };

        // set-hgrev-format
        let mut defrev = Vec::new();
        if !csetid.is_empty() {
            defrev.push("%h");
        }
        if !lrev.is_empty() {
            defrev.push("%r");
        }
        let revformat = self.style_s("hgrevformat").unwrap_or_else(|| defrev.join(":"));
        let init = Assoc::from_pairs(&[("localrev", lrev.as_str()), ("hash", csetid.as_str())]);
        let lrev = self.with_hook_com(init, |r| {
            if r.hook("set-hgrev-format", &[revformat.clone()]) == 0 {
                let (rv, h) = (r.st.hook_com.get("localrev").to_string(), r.st.hook_com.get("hash").to_string());
                zformat(&revformat, &[('r', rv.as_str()), ('h', h.as_str())])
            } else {
                r.st.hook_com.get("rev-replace").to_string()
            }
        });

        // set-branch-format
        let mut defbranch = vec!["%b"];
        if !lrev.is_empty() {
            defbranch.push("%r");
        }
        let branchformat = self.style_s("branchformat").unwrap_or_else(|| defbranch.join(":"));
        let init = Assoc::from_pairs(&[("branch", branch.as_str()), ("revision", lrev.as_str())]);
        let branch_out = self.with_hook_com(init, |r| {
            if r.hook("set-branch-format", &[branchformat.clone()]) == 0 {
                let (b, rv) = (r.st.hook_com.get("branch").to_string(), r.st.hook_com.get("revision").to_string());
                zformat(&branchformat, &[('b', b.as_str()), ('r', rv.as_str())])
            } else {
                r.st.hook_com.get("branch-replace").to_string()
            }
        });

        // bookmarks
        let mut hgbmstring = String::new();
        self.st.hook_com.clear();
        if self.style_t("get-bookmarks") && !csetid.is_empty() {
            if let Some(file) = &f.bookmarks_file {
                let mut marks = parse_hg_bookmarks(file, &csetid);
                let curbm = f.current_bookmark.clone().unwrap_or_default();
                if f.current_bookmark.is_some() {
                    self.st.hook_com.set("hg-active-bookmark", curbm.as_str());
                }
                self.st.set_array("hgbmarks", marks.clone());
                if self.hook("gen-hg-bookmark-string", &marks) == 0 {
                    if !curbm.is_empty() {
                        if let Some(i) = marks.iter().position(|m| *m == curbm) {
                            marks.remove(i);
                        }
                        marks.insert(0, format!("{curbm}*"));
                    }
                    hgbmstring = marks.join(", ");
                } else {
                    hgbmstring = self.st.hook_com.get("hg-bookmark-string").to_string();
                }
                self.st.hook_com.clear();
            }
        }

        // mq
        let mut hgmqstring = String::new();
        if let Some(mq) = f.mq.filter(|_| self.style_tt("get-mq")) {
            let applied = mq.status.as_deref().map(parse_mq_status).unwrap_or_default();
            self.st.set_array("mqpatches", applied.clone());
            let (mut guards, mut unapplied) = (Vec::new(), Vec::new());
            if self.style_t("get-unapplied") {
                if let Some(series) = &mq.series {
                    guards = mq.guards.as_deref().map(parse_mq_guards).unwrap_or_default();
                    unapplied = parse_mq_unapplied(series, &applied, &guards);
                }
            }
            self.st.set_array("mqguards", guards.clone());
            self.st.set_array("mqunapplied", unapplied.clone());
            let guards_string = if self.hook("gen-mqguards-string", &guards) == 0 {
                guards.join(",")
            } else {
                self.st.hook_com.get("guards-string").to_string()
            };
            let extra = Assoc::from_pairs(&[
                ("guards", guards_string.as_str()),
                ("guards-n", guards.len().to_string().as_str()),
            ]);
            hgmqstring = self.set_patch_format(&applied, &unapplied, &extra);
        }

        let misc: Vec<&str> = [hgmqstring.as_str(), hgbmstring.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        self.st.backend_misc.set("patches", hgmqstring.as_str());
        self.st.backend_misc.set("bookmarks", hgbmstring.as_str());
        self.formats(action, &branch_out, hgbase, "", changes, &lrev, &misc.join(";"));
        self.st.arrays.clear();
    }
}

// ---------------------------------------------------------------------
// svn — Backends/VCS_INFO_{detect,get_data}_svn
// ---------------------------------------------------------------------

/// `svn info` lines into a map: `IFS=: read a b; svninfo[${a// /_}]=${b## #}`.
fn parse_svn_info(out: &str) -> HashMap<String, String> {
    out.lines()
        .map(|l| match l.split_once(':') {
            Some((a, b)) => (a.replace(' ', "_"), b.trim_start_matches(' ').to_string()),
            None => (l.replace(' ', "_"), String::new()),
        })
        .collect()
}

/// The `misc` text get_data_svn passes for the two `svn info` failures it
/// special-cases (E155036 upgrade required, E155021 unsupported format).
/// `None` for any other failure.
fn svn_error_misc(code: Option<i32>, output: &str) -> Option<&'static str> {
    if code != Some(1) {
        return None;
    }
    let has = |prefix: &str| output.lines().any(|l| l.starts_with(prefix));
    if has("svn: E155036: ") {
        Some("working copy upgrade required")
    } else if has("svn: E155021: ") {
        Some("svn error")
    } else {
        None
    }
}

impl Run<'_> {
    pub(crate) fn detect_svn(&mut self) -> bool {
        if tool_bin(self.st.vcs_comm.get("cmd")).is_none() {
            return false;
        }
        bydir_detect(&self.cwd, ".svn", &["entries", "format", "wc.db"]).is_some()
    }

    /// `svn info --non-interactive [-- path]` parsed (stderr merged).
    fn svn_info(&self, path: Option<&Path>, dir: &Path) -> Option<(Option<i32>, String)> {
        let path_s = path.map(|p| p.to_string_lossy().into_owned());
        let mut args = vec!["info", "--non-interactive"];
        if let Some(p) = &path_s {
            args.push("--");
            args.push(p);
        }
        let o = self.tool(&args, dir, &[])?;
        Some((o.code, format!("{}{}", o.stdout, o.stderr)))
    }

    pub(crate) fn get_data_svn(&mut self) -> bool {
        self.st.vars.rrn.clear(); // `local rrn`
        let cwd = self.cwd.clone();
        let Some((code, text)) = self.svn_info(None, &cwd) else {
            return false;
        };
        if code != Some(0) {
            return match svn_error_misc(code, &text) {
                Some(misc) => {
                    let (base, revision) = ("?", "?");
                    self.with_hook_com(Assoc::default(), |r| {
                        r.formats("", "?", base, "", "", revision, misc)
                    });
                    true
                }
                None => false,
            };
        }
        let mut svninfo = parse_svn_info(&text);
        let cwdinfo = svninfo.clone();
        let mut svnbase = cwd.clone();
        if let Some(root) = svninfo.get("Working_Copy_Root_Path").cloned() {
            svnbase = PathBuf::from(root);
            if let Some((_, t)) = self.svn_info(Some(&svnbase), &cwd) {
                svninfo.extend(parse_svn_info(&t));
            }
        } else {
            while let Some(parent) = svnbase.parent().map(Path::to_path_buf) {
                if !parent.join(".svn").is_dir() {
                    break;
                }
                let parentinfo = match self.svn_info(Some(&parent), &cwd) {
                    Some((_, t)) => parse_svn_info(&t),
                    None => HashMap::new(),
                };
                if parentinfo.get("Repository_UUID") != svninfo.get("Repository_UUID") {
                    break;
                }
                svninfo = parentinfo;
                svnbase = parent;
                if svnbase == Path::new("/") {
                    break;
                }
            }
        }
        let base_s = svnbase.to_string_lossy().into_owned();
        self.st.vars.rrn = zsh_tail(&base_s).to_string();
        let url = svninfo.get("URL").cloned().unwrap_or_default();
        let url_tail = url.rsplit('/').next().unwrap_or("").to_string();
        let revision = cwdinfo.get("Revision").cloned().unwrap_or_default();
        let branch = self.set_branch_format(&url_tail, &revision).unwrap_or_default();
        self.formats("", &branch, &base_s, "", "", &revision, "");
        true
    }
}

// ---------------------------------------------------------------------
// Driver — the `vcs_info` function
// ---------------------------------------------------------------------

/// `vcs_info_msg_0_` as left by the previous run: `vcs_info` returns
/// without touching it when a `start-up` / `pre-get-data` hook asks to
/// stop (ret 1).
static LAST_MESSAGE: Mutex<Option<Message>> = Mutex::new(None);

fn remember(m: Option<Message>) -> Option<Message> {
    if let Ok(mut g) = LAST_MESSAGE.lock() {
        *g = m.clone();
    }
    m
}

fn previous_message() -> Option<Message> {
    LAST_MESSAGE
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .map(|m| Message {
            dirty: false,
            half_dirty: false,
            exports: Exports { kept: true, ..m.exports },
            ..m
        })
}

/// Backends that have a detect + get_data pair.
fn detect(run: &mut Run, vcs: &str) -> bool {
    match vcs {
        "hg" => run.detect_hg(),
        "svn" => run.detect_svn(),
        "bzr" => run.detect_bzr(),
        "cdv" => run.detect_cdv(),
        "cvs" => run.detect_cvs(),
        "darcs" => run.detect_darcs(),
        "fossil" => run.detect_fossil(),
        "mtn" => run.detect_mtn(),
        "p4" => run.detect_p4(),
        "svk" => run.detect_svk(),
        "tla" => run.detect_tla(),
        _ => false,
    }
}

fn get_data(run: &mut Run, vcs: &str) -> bool {
    match vcs {
        "hg" => run.get_data_hg(),
        "svn" => run.get_data_svn(),
        "bzr" => run.get_data_bzr(),
        "cdv" => run.get_data_cdv(),
        "cvs" => run.get_data_cvs(),
        "darcs" => run.get_data_darcs(),
        "fossil" => run.get_data_fossil(),
        "mtn" => run.get_data_mtn(),
        "p4" => run.get_data_p4(),
        "svk" => run.get_data_svk(),
        "tla" => run.get_data_tla(),
        _ => false,
    }
}

/// `vcs_info`: the zstyle `enable` backends in order, the first detected
/// backend wins (vcs_info:105-114).
pub(crate) fn vcs_info(host: &dyn HookHost, cwd: PathBuf, pwd: String) -> Option<Message> {
    let mut run = Run::new(host, cwd, pwd);
    run.st.vars.maxexports = 0;

    // VCS_INFO_hook "start-up": 1 = keep the old message, 2 = no vcs.
    match run.hook("start-up", &[]) {
        1 => return previous_message(),
        2 => {
            run.read_maxexports();
            run.set_nvcs();
            return remember(Some(run.message()));
        }
        _ => {}
    }
    let mut enabled = run.style_a("enable");
    if enabled.is_empty() {
        enabled.push("all".to_string());
    }
    let eq = |b: &String, w: &str| b.eq_ignore_ascii_case(w);
    if enabled.iter().any(|b| eq(b, "none")) {
        return remember(run.nvcs_if_shown());
    }
    let mut disabled: Vec<String> = Vec::new();
    if enabled.iter().any(|b| eq(b, "all")) {
        enabled = BACKENDS.iter().filter(|b| **b != "git").map(|b| b.to_string()).collect();
        disabled = run.style_a("disable");
    }
    for pat in run.style_a("disable-patterns") {
        if subst_pattern_matches(&pat, &run.pwd) {
            run.read_maxexports();
            return remember(run.nvcs_if_shown());
        }
    }
    run.read_maxexports();

    let mut found = false;
    for vcs in &enabled {
        if disabled.contains(vcs) {
            continue;
        }
        if !BACKENDS.contains(&vcs.as_str()) || vcs == "git" {
            tracing::debug!(target: "p10k", backend = %vcs, "vcs backend unknown to vcs_info");
            continue;
        }
        run.st.vcs_comm.clear();
        run.st.vars.vcs = vcs.clone();
        run.get_cmd();
        if detect(&mut run, vcs) {
            found = true;
            break;
        }
    }
    if !found {
        // `vcs='-quilt-'; quiltmode='standalone'; VCS_INFO_quilt standalone
        // || VCS_INFO_set --nvcs`
        run.st.vars.vcs = "-quilt-".into();
        run.st.vars.quiltmode = "standalone".into();
        if run.quilt("standalone").0 != 0 {
            run.set_nvcs();
        }
        return remember(Some(run.message()));
    }

    match run.hook("pre-get-data", &[]) {
        1 => return previous_message(),
        2 => {
            run.set_nvcs();
            return remember(Some(run.message()));
        }
        _ => {}
    }
    let vcs = run.st.vars.vcs.clone();
    let ok = run.with_hook_com(Assoc::default(), |r| get_data(r, &vcs));
    if !ok {
        run.set_nvcs();
    }
    remember(Some(run.message()))
}

impl Run<'_> {
    /// `[[ -n ${vcs_info_msg_0_} ]] && VCS_INFO_set --nvcs`: the early
    /// exits clear a message that is still showing.
    fn nvcs_if_shown(&mut self) -> Option<Message> {
        let showing = getsparam("vcs_info_msg_0_").is_some_and(|m| !m.is_empty());
        showing.then(|| {
            self.set_nvcs();
            self.message()
        })
    }
}

/// `VCS_INFO_set`: publish `msgs` as the `vcs_info_msg_<N>_` globals.
fn export_messages(e: &Exports) {
    if e.kept {
        return;
    }
    let name = |i: usize| format!("vcs_info_msg_{i}_");
    let current = |i: usize| getsparam(&name(i)).unwrap_or_default();
    if e.nvcs {
        for i in 0..e.maxexports.max(1) {
            setsparam(&name(i), "");
        }
    }
    if e.msgs.is_empty() {
        return;
    }
    for (i, msg) in e.msgs.iter().enumerate() {
        setsparam(&name(i), msg);
    }
    for j in e.msgs.len()..=e.maxexports {
        if !current(j).is_empty() {
            setsparam(&name(j), "");
        }
    }
}

/// Non-git backends from `POWERLEVEL9K_VCS_BACKENDS` -> segment.
pub(crate) fn vcs_info_segments(backends: &[String]) -> Vec<Segment> {
    let cwd = physical_cwd();
    let cfg = P10kCfg::load();
    // p10k:4185 `zstyle ':vcs_info:*' enable ${backends}`, with the init
    // styles it depends on.
    sync_global_styles(&cfg, backends);
    let host = PromptHost { cfg, cwd: cwd.clone(), table: None, svn_status: None };
    let pwd = getsparam("PWD").unwrap_or_else(|| cwd.to_string_lossy().into_owned());
    let Some(msg) = vcs_info(&host, cwd, pwd) else { return Vec::new() };
    export_messages(&msg.exports);
    build_segment(msg)
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
        .as_deref()
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
    use crate::extensions::p10k::vcs_hooks::test_host::MockHost;

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

    fn cfg(show_changeset: bool) -> P10kCfg {
        P10kCfg {
            icons: icons(),
            show_changeset,
            hash_len: 8,
            hide_branch_icon: false,
            action_fg: "1".into(),
            git_hooks: vec!["vcs-detect-changes".into(), "git-untracked".into()],
            hg_hooks: vec!["vcs-detect-changes".into()],
            svn_hooks: vec!["vcs-detect-changes".into(), "svn-detect-changes".into()],
        }
    }

    /// A zstyle table holding only the theme's styles.
    fn themed(cfg: &P10kCfg) -> style_table {
        let mut table = style_table::new();
        install_theme_styles(&mut table, &theme_styles(cfg));
        table
    }

    fn host(show_changeset: bool, svn_status: &str) -> PromptHost {
        let cfg = cfg(show_changeset);
        PromptHost {
            table: Some(themed(&cfg)),
            cfg,
            cwd: PathBuf::from("/work/proj/sub"),
            svn_status: Some(svn_status.to_string()),
        }
    }

    fn run_for<'a>(h: &'a dyn HookHost, vcs: &str) -> Run<'a> {
        let mut r = Run::new(h, PathBuf::from("/work/proj/sub"), "/work/proj/sub".into());
        r.st.vars.vcs = vcs.into();
        r.st.vars.maxexports = 2;
        r
    }

    fn hg_facts() -> HgFacts {
        HgFacts {
            csetid: "5f3a9c1e7b20".into(),
            lrev: "142".into(),
            branch_file: "default".into(),
            ..Default::default()
        }
    }

    fn hg_run(h: &dyn HookHost, facts: HgFacts) -> Message {
        let mut r = run_for(h, "hg");
        r.st.vars.rrn = "proj".into();
        r.hg_finish("/work/proj", facts);
        r.message()
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
        let status = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2:first.patch\n\n\
                      0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f:second.patch\n";
        assert_eq!(
            parse_mq_status(status),
            vec!["second.patch".to_string(), "first.patch".to_string()]
        );
    }

    #[test]
    fn mq_series_honours_applied_and_guards() {
        let series = "# comment\n\
                      applied.patch\n\
                      plain.patch\n\
                      \n\
                      posonly.patch #+stable\n\
                      negonly.patch #-stable\n\
                      both.patch #+stable #-broken\n";
        let applied = vec!["applied.patch".to_string()];
        // no guard active: pos-guarded patches are excluded, neg-guarded kept
        assert_eq!(
            parse_mq_unapplied(series, &applied, &[]),
            vec!["plain.patch", "negonly.patch"]
        );
        // `stable` active: pos kept, neg excluded, `both` kept (its neg guard is inactive)
        let active = vec!["stable".to_string()];
        assert_eq!(
            parse_mq_unapplied(series, &applied, &active),
            vec!["plain.patch", "posonly.patch", "both.patch"]
        );
        // `broken` active: `both` is excluded by its negative guard
        let active = vec!["broken".to_string(), "stable".to_string()];
        assert_eq!(
            parse_mq_unapplied(series, &applied, &active),
            vec!["plain.patch", "posonly.patch"]
        );
        assert_eq!(parse_mq_guards("zeta\n\nalpha\n"), vec!["alpha", "zeta"]);
    }

    #[test]
    fn hg_clean_and_dirty_messages() {
        let h = host(false, "");
        let m = hg_run(&h, hg_facts());
        assert_eq!(m.text, "B:default");
        assert!(!m.dirty && !m.half_dirty);
        assert_eq!(m.icon_key.as_deref(), Some("VCS_HG_ICON"));
        assert_eq!(m.exports.msgs, vec!["B:default"]);
        let m = hg_run(&h, HgFacts { lrev: "142+".into(), csetid: "5f3a9c1e7b20+".into(), ..hg_facts() });
        assert_eq!(m.text, "B:default U");
        assert!(m.dirty);
    }

    #[test]
    fn hg_changeset_prefix_bookmarks_mq_and_action() {
        let h = host(true, "");
        let facts = HgFacts {
            branch_file: "stable".into(),
            bookmarks_file: Some(
                "5f3a9c1e7b20aaaaaaaaaaaaaaaaaaaaaaaaaaaa a\n5f3a9c1e7b20bbbbbbbbbbbbbbbbbbbbbbbbbbbb b\n"
                    .into(),
            ),
            mq: Some(MqFiles {
                status: Some("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2:p.patch\n".into()),
                ..Default::default()
            }),
            ..hg_facts()
        };
        // %0.8i truncates "5f3a9c1e7b20:142" to 8 bytes.
        let m = hg_run(&h, facts);
        assert_eq!(m.text, "C:5f3a9c1e B:stablep.patch (1 applied); K:a b");
        let m = hg_run(&h, HgFacts { merging: true, ..hg_facts() });
        assert_eq!(m.text, "B:default %F{1}| merging%f");
        // rebasing beats merging
        let m = hg_run(&h, HgFacts { merging: true, rebasing: true, ..hg_facts() });
        assert_eq!(m.text, "B:default %F{1}| rebasing%f");
    }

    #[test]
    fn hg_active_bookmark_goes_first_with_star_without_a_bookmark_hook() {
        let h = MockHost::default()
            .with_style(":vcs_info:hg:default:proj", "get-bookmarks", &["true"])
            .with_style(":vcs_info:hg:default:proj", "formats", &["%m"]);
        let mut r = run_for(&h, "hg");
        r.st.vars.rrn = "proj".into();
        let facts = HgFacts {
            bookmarks_file: Some(
                "5f3a9c1e7b20aaaaaaaaaaaaaaaaaaaaaaaaaaaa one\n5f3a9c1e7b20bbbbbbbbbbbbbbbbbbbbbbbbbbbb two\n"
                    .into(),
            ),
            current_bookmark: Some("two".into()),
            ..hg_facts()
        };
        r.hg_finish("/work/proj", facts);
        assert_eq!(r.message().text, "two*, one");
    }

    #[test]
    fn hg_flavour_loses_visual_identifier() {
        let h = host(false, "");
        let mut r = run_for(&h, "hg-git");
        r.st.vars.rrn = "proj".into();
        r.hg_finish("/work/proj", HgFacts { lrev: "1+".into(), ..hg_facts() });
        let m = r.message();
        assert_eq!(m.icon_key, None, "only hook_com[vcs] == hg|svn sets the identifier");
        assert!(m.dirty, "the dirty flag depends on staged/unstaged only");
    }

    #[test]
    fn hg_gen_hook_ret_replaces_bookmark_string() {
        fn replace(st: &mut HookState) -> i32 {
            st.hook_com.set("hg-bookmark-string", " custom");
            st.ret = 1;
            0
        }
        let mut h = MockHost::default()
            .with_style(":vcs_info:hg+gen-hg-bookmark-string:default:proj", "hooks", &["mine"])
            .with_style(":vcs_info:hg:default:proj", "get-bookmarks", &["true"])
            .with_style(":vcs_info:hg:default:proj", "formats", &["%m"]);
        h.functions = vec!["+vi-mine".into()];
        h.behaviour.insert("+vi-mine".into(), replace);
        let mut r = run_for(&h, "hg");
        r.st.vars.rrn = "proj".into();
        let facts = HgFacts {
            bookmarks_file: Some("5f3a9c1e7b20aaaa one\n".into()),
            ..hg_facts()
        };
        r.hg_finish("/work/proj", facts);
        assert_eq!(r.message().text, " custom");
        let calls = h.calls.borrow();
        assert_eq!(calls[0], ("+vi-mine".to_string(), vec!["one".to_string()]));
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
        assert_eq!(svn_error_misc(Some(1), up), Some("working copy upgrade required"));
        assert_eq!(svn_error_misc(Some(1), "svn: E155021: bad\n"), Some("svn error"));
        assert_eq!(svn_error_misc(Some(2), up), None);
        assert_eq!(svn_error_misc(Some(1), "svn: E155007: not a working copy\n"), None);
    }

    fn svn_run(h: &dyn HookHost, revision: &str) -> Message {
        let mut r = run_for(h, "svn");
        r.st.vars.rrn = "wc".into();
        r.formats("", "trunk:4711", "/home/u/wc", "", "", revision, "");
        r.message()
    }

    #[test]
    fn svn_messages_follow_hooks_and_state() {
        // Clean tree, no changeset prefix: empty message, segment hidden.
        let clean = svn_run(&host(false, ""), "4711");
        assert_eq!(clean.text, "");
        assert!(build_segment(clean).is_empty());
        // Untracked only: UNTRACKED (half dirty), unstaged gets the icon.
        let m = svn_run(&host(false, "?       new.txt\n"), "4711");
        assert_eq!((m.text.as_str(), m.dirty, m.half_dirty), (" T", false, true));
        // Everything: staged first, then unstaged (untracked before modified).
        let m = svn_run(&host(true, "?  a\nM  b\nA  c\n"), "4711");
        assert_eq!(m.text, "C:4711  S T U");
        assert!(m.dirty);
        assert_eq!(m.icon_key.as_deref(), Some("VCS_SVN_ICON"));
    }

    #[test]
    fn svn_error_message_shows_question_mark_revision() {
        let h = host(true, "");
        let mut r = run_for(&h, "svn");
        r.formats("", "?", "?", "", "", "?", "working copy upgrade required");
        assert_eq!(r.message().text, "C:? ");
    }

    fn lookup(t: &style_table, ctx: &str, style: &str) -> Vec<String> {
        t.get_match(ctx, style).map(|(vals, _)| vals).unwrap_or_default()
    }

    #[test]
    fn theme_styles_match_vcs_info_init() {
        let t = themed(&cfg(true));
        let d = |ctx: &str, style: &str| lookup(&t, ctx, style);
        assert_eq!(d(":vcs_info:hg:default:r", "formats"), vec!["C:%0.8i %b%c%u%m"]);
        assert_eq!(d(":vcs_info:svn:default:r", "formats"), vec!["C:%0.8i %c%u"]);
        assert_eq!(d(":vcs_info:bzr:default:r", "actionformats"), vec!["%b %F{1}| %a%f"]);
        assert_eq!(
            d(":vcs_info:svn:default:r", "actionformats"),
            vec!["C:%0.8i %c%u %F{1}| %a%f"]
        );
        assert_eq!(d(":vcs_info:bzr:default:r", "stagedstr"), vec![" S"]);
        assert_eq!(d(":vcs_info:hg-git:default:r", "branchformat"), vec!["B:%b"]);
        assert!(d(":vcs_info:bzr:default:r", "branchformat").is_empty());
        assert_eq!(d(":vcs_info:hg:default:r", "get-revision"), vec!["true"]);
        assert_eq!(d(":vcs_info:bzr:default:r", "get-revision"), vec!["true"]); // SHOW_CHANGESET
        assert_eq!(d(":vcs_info:bzr:default:r", "check-for-changes"), vec!["true"]);
        assert_eq!(
            d(":vcs_info:hg+set-message:default:r", "hooks"),
            vec!["vcs-detect-changes"]
        );
        assert_eq!(
            d(":vcs_info:svn+set-message:default:r", "hooks"),
            vec!["vcs-detect-changes", "svn-detect-changes"]
        );
        assert_eq!(
            d(":vcs_info:git+set-message:default:r", "hooks"),
            vec!["vcs-detect-changes", "git-untracked"]
        );
        assert_eq!(
            d(":vcs_info:hg+gen-hg-bookmark-string:default:r", "hooks"),
            vec!["hg-bookmarks"]
        );
        assert!(d(":vcs_info:bzr+set-message:default:r", "hooks").is_empty());
        assert!(d(":vcs_info-static_hooks:set-message", "hooks").is_empty());
        let mut hidden = cfg(false);
        hidden.hide_branch_icon = true;
        let t = themed(&hidden);
        assert_eq!(lookup(&t, ":vcs_info:hg:default:r", "branchformat"), vec!["%b"]);
        assert_eq!(lookup(&t, ":vcs_info:bzr:default:r", "get-revision"), vec!["false"]);
    }

    #[test]
    fn a_set_but_empty_theme_hook_list_still_overrides_less_specific_styles() {
        let mut c = cfg(false);
        c.hg_hooks = Vec::new();
        let mut t = style_table::new();
        t.set(":vcs_info:*", "hooks", vec!["user-hook".into()], None);
        install_theme_styles(&mut t, &theme_styles(&c));
        // `hg*+set-message:*` has more colon components than `:vcs_info:*`
        assert!(lookup(&t, ":vcs_info:hg+set-message:default:r", "hooks").is_empty());
        assert_eq!(lookup(&t, ":vcs_info:bzr+set-message:default:r", "hooks"), vec!["user-hook"]);
    }

    #[test]
    fn user_and_theme_styles_rank_by_zstyle_specificity() {
        let c = cfg(false);
        let mut t = style_table::new();
        // set before the theme's init: an identical pattern is replaced
        t.set(":vcs_info:*", "formats", vec!["user-all".into()], None);
        // fewer colon components than the theme's `hg*:*`: loses on hg
        t.set(":vcs_info:*", "branchformat", vec!["user-branch".into()], None);
        // equal weight to the theme's `svn*:*` (`s*` and `svn*` both score
        // 1) and set first: stays ahead of it
        t.set(":vcs_info:s*:*", "actionformats", vec!["user-early".into()], None);
        install_theme_styles(&mut t, &theme_styles(&c));
        // more specific than the theme's `svn*:*` (`svn` scores 2): wins
        t.set(":vcs_info:svn:*", "unstagedstr", vec!["user-svn".into()], None);

        let svn = ":vcs_info:svn:default:r";
        let bzr = ":vcs_info:bzr:default:r";
        assert_eq!(lookup(&t, bzr, "formats"), vec!["%b%c%u%m"], "same pattern: theme replaces");
        assert_eq!(lookup(&t, svn, "formats"), vec!["%c%u"]);
        assert_eq!(lookup(&t, ":vcs_info:hg:default:r", "branchformat"), vec!["B:%b"]);
        assert_eq!(lookup(&t, bzr, "branchformat"), vec!["user-branch"]);
        assert_eq!(lookup(&t, svn, "actionformats"), vec!["user-early"]);
        assert_eq!(lookup(&t, svn, "unstagedstr"), vec!["user-svn"]);
        assert_eq!(lookup(&t, bzr, "unstagedstr"), vec![" U"]);
    }

    #[test]
    fn a_user_style_set_after_init_replaces_the_themes_identical_pattern() {
        let mut t = themed(&cfg(false));
        t.set(":vcs_info:*", "formats", vec!["later".into()], None);
        assert_eq!(lookup(&t, ":vcs_info:bzr:default:r", "formats"), vec!["later"]);
    }

    #[test]
    fn exports_publish_every_message_and_clear_stale_ones() {
        let e = Exports { msgs: vec!["a".into(), "b".into()], maxexports: 3, nvcs: false, kept: false };
        assert_eq!(e.msgs.len(), 2);
        let h = host(false, "");
        let mut m = Run::new(&h, PathBuf::from("/w"), "/w".into());
        m.st.vars.maxexports = 3;
        m.st.msgs = vec!["a".into(), "b".into()];
        assert_eq!(m.message().exports, e);
        m.set_nvcs();
        assert!(m.message().exports.nvcs);
    }

    #[test]
    fn user_hooks_edit_hook_com_between_backend_and_format() {
        fn post_backend(st: &mut HookState) -> i32 {
            st.hook_com.set("branch", "from-hook");
            0
        }
        fn set_message(st: &mut HookState) -> i32 {
            // returning ret=1 makes vcs_info use hook_com[message]
            st.hook_com.set("message", "custom message");
            st.ret = 1;
            0
        }
        let mut h = MockHost::default()
            .with_style(":vcs_info:bzr:default:repo", "formats", &["[%b]"])
            .with_style(":vcs_info:bzr+post-backend:default:repo", "hooks", &["pb"]);
        h.functions = vec!["+vi-pb".into(), "+vi-sm".into()];
        h.behaviour.insert("+vi-pb".into(), post_backend);
        let mut r = run_for(&h, "bzr");
        r.st.vars.rrn = "repo".into();
        r.formats("", "orig", "/work/repo", "", "", "7", "");
        assert_eq!(r.message().text, "[from-hook]");

        h.styles.insert(
            (":vcs_info:bzr+set-message:default:repo".into(), "hooks".into()),
            vec!["sm".into()],
        );
        h.behaviour.insert("+vi-sm".into(), set_message);
        let mut r = run_for(&h, "bzr");
        r.st.vars.rrn = "repo".into();
        r.formats("", "orig", "/work/repo", "", "", "7", "");
        assert_eq!(r.message().text, "custom message");
        // set-message receives (index, format)
        let calls = h.calls.borrow();
        let sm = calls.iter().find(|c| c.0 == "+vi-sm").unwrap();
        assert_eq!(sm.1, vec!["0".to_string(), "[%b]".to_string()]);
    }

    #[test]
    fn formats_pipeline_defaults_and_staged_strings() {
        let h = MockHost::default();
        let mut r = run_for(&h, "bzr");
        r.st.vars.rrn = "repo".into();
        // no formats style: vcs_info's own fallback; staged/unstaged become S/U
        r.formats("", "main", "/work/repo", "x", "y", "9", "");
        assert_eq!(r.message().text, " (bzr)-[main]US-");
        let mut r = run_for(&h, "bzr");
        r.st.vars.rrn = "repo".into();
        r.formats("merging", "main", "/work/repo", "", "", "9", "");
        assert_eq!(r.message().text, " (bzr)-[main|merging]-");
    }

    #[test]
    fn formats_specs_cover_every_zformat_letter() {
        let h = MockHost::default().with_style(
            ":vcs_info:bzr:default:repo",
            "formats",
            &["a=%a b=%b c=%c i=%i m=%m r=%r s=%s u=%u R=%R S=%S"],
        );
        let mut r = run_for(&h, "bzr");
        r.st.vars.rrn = "repo".into();
        r.formats("", "br", "/work/proj", "", "", "42", "ms");
        assert_eq!(
            r.message().text,
            "a= b=br c= i=42 m=ms r=proj s=bzr u= R=/work/proj S=sub"
        );
    }

    #[test]
    fn max_exports_truncates_and_nvcs_keeps_one_less() {
        let h = MockHost::default()
            .with_style(":vcs_info:bzr:default:repo", "formats", &["one", "two", "three"])
            .with_style(":vcs_info:-quilt-:default:-all-", "nvcsformats", &["n0", "n1", "n2"]);
        let mut r = run_for(&h, "bzr");
        r.st.vars.rrn = "repo".into();
        r.formats("", "b", "/work/repo", "", "", "", "");
        assert_eq!(r.st.msgs, vec!["one", "two"]);
        let mut r = run_for(&h, "-quilt-");
        r.st.vars.rrn = "-all-".into();
        r.set_nvcs();
        assert_eq!(r.st.msgs, vec!["n0"]);
    }

    #[test]
    fn patch_format_default_hooks_and_escaping() {
        let h = MockHost::default();
        let mut r = run_for(&h, "hg");
        r.st.vars.rrn = "proj".into();
        let extra = Assoc::from_pairs(&[("guards", "g1,g2"), ("guards-n", "2")]);
        let applied = vec!["50%.patch".to_string(), "first.patch".to_string()];
        assert_eq!(r.set_patch_format(&applied, &[], &extra), "50%%.patch (2 applied)");
        assert_eq!(r.set_patch_format(&[], &[], &extra), "no patch applied");

        let h = MockHost::default()
            .with_style(":vcs_info:hg:default:proj", "patch-format", &["%p/%u/%n/%c/%a/%g/%G"]);
        let mut r = run_for(&h, "hg");
        r.st.vars.rrn = "proj".into();
        r.st.set_array("mqguards", vec!["x".into(), "y".into()]);
        let unapplied = vec!["u1".to_string(), "u2".to_string(), "u3".to_string()];
        assert_eq!(
            r.set_patch_format(&["top".to_string()], &unapplied, &extra),
            "top/3/1/3/4/g1,g2/2"
        );
    }

    #[test]
    fn set_branch_format_defaults_hooks_and_missing_rrn() {
        let h = MockHost::default();
        let mut r = run_for(&h, "p4");
        r.st.vars.rrn.clear();
        assert_eq!(r.set_branch_format("client", "123"), None); // rrn empty
        r.st.vars.rrn = "ws".into();
        assert_eq!(r.set_branch_format("client", "123").as_deref(), Some("client:123"));

        fn replace(st: &mut HookState) -> i32 {
            st.hook_com.set("branch-replace", "replaced");
            st.ret = 1;
            0
        }
        let mut h = MockHost::default()
            .with_style(":vcs_info:p4+set-branch-format:default:ws", "hooks", &["bf"]);
        h.functions = vec!["+vi-bf".into()];
        h.behaviour.insert("+vi-bf".into(), replace);
        let mut r = run_for(&h, "p4");
        r.st.vars.rrn = "ws".into();
        r.st.hook_com.set("keep", "me");
        assert_eq!(r.set_branch_format("client", "123").as_deref(), Some("replaced"));
        assert_eq!(r.st.hook_com.get("keep"), "me", "caller's hook_com is restored");
        assert_eq!(h.calls.borrow()[0].1, vec!["%b:%r".to_string()]);
    }

    #[test]
    fn reposub_is_relative_to_the_base() {
        let cwd = Path::new("/work/proj/sub/dir");
        assert_eq!(reposub("/work/proj", cwd), "sub/dir");
        assert_eq!(reposub("/work/proj/", cwd), "sub/dir");
        assert_eq!(reposub("/work/proj/sub/dir", cwd), ".");
        assert_eq!(reposub("/elsewhere", cwd), ".");
    }

    #[test]
    fn build_segment_state_and_hiding() {
        let m = |dirty, half| Message {
            text: "x".into(),
            dirty,
            half_dirty: half,
            icon_key: None,
            exports: Exports::default(),
        };
        assert_eq!(build_segment(m(true, true))[0].state.as_deref(), Some("MODIFIED"));
        assert_eq!(build_segment(m(false, true))[0].state.as_deref(), Some("UNTRACKED"));
        assert_eq!(build_segment(m(false, false))[0].state.as_deref(), Some("CLEAN"));
    }

    #[test]
    fn hexdump_is_lowercase_pairs() {
        let dir = std::env::temp_dir().join(format!("p10k-hexdump-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("dirstate");
        std::fs::write(&f, [0x00u8, 0xab, 0x0f, 0xff, 0x10]).unwrap();
        assert_eq!(hexdump(&f, 4).as_deref(), Some("00ab0fff"));
        assert_eq!(hexdump(&f, 20).as_deref(), Some("00ab0fff10"));
        assert_eq!(hexdump(&dir.join("missing"), 4), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn revision_truncation_through_zformat() {
        assert_eq!(zformat("%0.3i|", &[('i', "abcdef")]), "abc|");
        assert_eq!(zformat("%0.8i|", &[('i', "abc")]), "abc|");
    }
}
