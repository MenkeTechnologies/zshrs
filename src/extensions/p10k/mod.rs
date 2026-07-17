//! Native powerlevel10k prompt engine — extension; no zsh C counterpart.
//!
//! Replaces the powerlevel10k zsh theme (~13k lines of metaprogrammed
//! zsh at ~/forkedRepos/powerlevel10k) with an in-process Rust segment
//! engine. The theme file is the SPEC: segment semantics are ported
//! from `internal/p10k.zsh` and cited as `// p10k:NNN`. The user's
//! `.p10k.zsh` config still sources normally — it is ~600 plain
//! `typeset -g POWERLEVEL9K_*` assignments that land in the paramtab,
//! which `config::p9k_param` reads with p10k's own fallback chain.
//!
//! Activation: sourcing `powerlevel10k.zsh-theme` is intercepted at
//! the builtin-dispatch layer (`maybe_intercept_theme_source`) — the
//! zsh theme never executes; the engine renders PROMPT/RPROMPT at
//! preprompt time instead (`preprompt_render`, called after the
//! `precmd` hook so user precmd state is fresh). Removing the
//! intercept call restores the stock zsh theme path untouched.
//!
//! This also absorbs gitstatusd: `git.rs` computes git status
//! in-process (no C++ daemon, no fork per prompt for the cached case).

pub mod config;
pub mod expansion;
pub mod git;
pub mod icons;
pub mod render;
pub mod segments_core;
pub mod segments_env;
pub mod segments_extra;
pub mod segments_powerline;
pub mod segments_sys;
pub mod segments_zshrs;
pub mod transient;

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

/// Engine on/off. Flipped by the theme-source intercept; never reset
/// (a session that loaded p10k keeps the native engine for life,
/// matching the zsh theme's own irreversibility).
static ENGINE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn engine_active() -> bool {
    ENGINE_ACTIVE.load(Ordering::Relaxed)
}

/// Monotonic start-of-command stamp (millis since an arbitrary epoch),
/// written by the preexec site in init.rs. 0 = no command started yet.
/// p10k:_p9k_preexec sets `_p9k__timer_start=EPOCHREALTIME` — same
/// contract, Rust-side so no shell hook is needed.
static EXEC_START_MS: AtomicU64 = AtomicU64::new(0);
/// Duration of the last finished foreground command, millis. u64::MAX =
/// nothing measured yet this session.
static EXEC_LAST_MS: AtomicU64 = AtomicU64::new(u64::MAX);

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Called from the preexec dispatch site right before a foreground
/// command runs (init.rs).
pub fn note_exec_start() {
    if engine_active() {
        EXEC_START_MS.store(now_ms(), Ordering::Relaxed);
    }
}

/// Called from preprompt() BEFORE the precmd hook fires: closes the
/// timing window (p10k:_p9k_on_expand reads `_p9k__timer_start` at
/// prompt build) and snapshots `$?` before precmd commands clobber it
/// (p10k:_p9k_save_status).
pub fn note_command_finished(last_status: i64) {
    if !engine_active() {
        return;
    }
    LAST_STATUS.store(last_status, Ordering::Relaxed);
    let start = EXEC_START_MS.swap(0, Ordering::Relaxed);
    if start != 0 {
        EXEC_LAST_MS.store(now_ms().saturating_sub(start), Ordering::Relaxed);
    }
}

/// Duration of the last foreground command, for the
/// command_execution_time segment. None until the first command ends.
pub fn last_exec_duration() -> Option<std::time::Duration> {
    match EXEC_LAST_MS.load(Ordering::Relaxed) {
        u64::MAX => None,
        ms => Some(std::time::Duration::from_millis(ms)),
    }
}

/// `$?` as it stood when the last foreground command finished —
/// captured before precmd hooks run, so the status/prompt_char
/// segments can't be poisoned by precmd's own commands.
/// p10k:_p9k_save_status does exactly this.
static LAST_STATUS: AtomicI64 = AtomicI64::new(0);

pub fn last_status() -> i64 {
    LAST_STATUS.load(Ordering::Relaxed)
}

/// Intercept `source <path>` / `. <path>` at builtin dispatch.
///
/// Returns `Some(status)` when the sourced file IS the powerlevel10k
/// theme entry point — the caller must skip the real source and use
/// the status. Returns `None` for every other file (including the
/// user's `.p10k.zsh` config, which must source normally so its
/// `POWERLEVEL9K_*` typesets reach the paramtab).
pub fn maybe_intercept_theme_source(args: &[String]) -> Option<i32> {
    let path = args.first()?;
    // powerlevel10k.zsh-theme and its plugin-manager aliases
    // (powerlevel10k.plugin.zsh, prompt_powerlevel10k_setup) are the
    // only entry points that execute internal/p10k.zsh (theme:1-83).
    let base = path.rsplit('/').next().unwrap_or(path);
    if !matches!(
        base,
        "powerlevel10k.zsh-theme" | "powerlevel10k.plugin.zsh" | "prompt_powerlevel10k_setup"
    ) {
        return None;
    }
    ENGINE_ACTIVE.store(true, Ordering::Relaxed);
    tracing::info!(target: "p10k", %path, "native p10k engine activated (theme source intercepted)");
    Some(0)
}

/// Intercept commands the zsh theme would have defined as functions
/// (`p10k`, `p10k-instant-prompt-finalize`, …). With the theme never
/// executing, calls from .zshrc / zpwr to those names would otherwise
/// die "command not found". Accept them silently; configuration
/// reload is unnecessary because the engine reads the paramtab live
/// on every render.
pub fn maybe_intercept_command(name: &str, args: &[String]) -> Option<i32> {
    if !engine_active() {
        return None;
    }
    match name {
        // The `p10k(){ zshrs-p10k-api "$@" }` stub (registered at
        // theme intercept) forwards here. p10k:8600+ `function p10k()`.
        "zshrs-p10k-api" | "p10k" | "p10k-instant-prompt-finalize" => Some(p10k_api(args)),
        _ => None,
    }
}

thread_local! {
    /// Collects `p10k segment …` emissions while a user
    /// `prompt_<name>` function runs (p10k:8654-8698 `_p9k_segment`).
    /// None = no user segment fn active; `p10k segment` outside one
    /// warns like the theme does (p10k:8659).
    static USER_SEGMENT_SINK: std::cell::RefCell<Option<Vec<render::Segment>>> =
        const { std::cell::RefCell::new(None) };
}

/// `p10k <command>` API (p10k:8600+). `segment` collects into the
/// active sink; `display`/`reload`/`refresh` are no-ops (live paramtab
/// reads); unknown subcommands warn on stderr like the theme.
fn p10k_api(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("segment") => {
            // p10k:8663-8687 — `p10k segment [-t text] [-i icon]
            // [-f fg] [-b bg] [-s state] [-c cond] [-e]`.
            let mut text = String::new();
            let mut icon = String::new();
            let mut fg = String::new();
            let mut bg = String::new();
            let mut state: Option<String> = None;
            let mut cond = true;
            let mut i = 1;
            while i < args.len() {
                let take = |i: &mut usize| -> String {
                    *i += 1;
                    args.get(*i).cloned().unwrap_or_default()
                };
                match args[i].as_str() {
                    "-t" => text = take(&mut i),
                    "-i" => icon = take(&mut i),
                    "-f" => fg = take(&mut i),
                    "-b" => bg = take(&mut i),
                    "-s" => state = Some(take(&mut i)),
                    // p10k:8680 — `-c cond`: empty expansion hides.
                    "-c" => cond = !take(&mut i).is_empty(),
                    // -e (expand text as prompt escapes) / -r (icon is
                    // literal): text passes through as prompt escapes
                    // already, so both are accepted and ignored.
                    "-e" | "-r" => {}
                    other => {
                        tracing::debug!(target: "p10k", %other, "p10k segment: unknown flag ignored")
                    }
                }
                i += 1;
            }
            USER_SEGMENT_SINK.with(|s| {
                let mut slot = s.borrow_mut();
                match slot.as_mut() {
                    Some(v) => {
                        if cond {
                            v.push(render::Segment {
                                name: String::new(), // filled by the runner
                                state,
                                content: text,
                                icon: if icon.is_empty() { None } else { Some(icon) },
                                fg,
                                bg,
                            });
                        }
                        0
                    }
                    None => {
                        // p10k:8659 — "segment: can be called only
                        // during prompt rendering".
                        eprintln!("zshrs: p10k: segment: can be called only during prompt rendering");
                        1
                    }
                }
            })
        }
        // Live paramtab reads make these no-ops.
        Some("reload") | Some("display") | Some("refresh") | None => 0,
        Some("finalize") => 0,
        Some(other) => {
            tracing::debug!(target: "p10k", %other, "p10k API subcommand absorbed");
            0
        }
    }
}

/// Run a user-defined `prompt_<name>` shell function as a segment
/// builder (p10k:8654+ custom segment protocol): arm the sink, call
/// the function through the executor, collect what `p10k segment`
/// deposited. Returns None when no such function exists (the caller
/// then logs "not implemented").
fn run_user_segment_fn(base: &str) -> Option<Vec<render::Segment>> {
    let fname = format!("prompt_{base}");
    crate::ported::utils::getshfunc(&fname)?;
    USER_SEGMENT_SINK.with(|s| *s.borrow_mut() = Some(Vec::new()));
    let status = crate::fusevm_bridge::try_with_executor(|exec| {
        exec.execute_script(&fname).unwrap_or(-1)
    })
    .unwrap_or(-1);
    let segs = USER_SEGMENT_SINK.with(|s| s.borrow_mut().take()).unwrap_or_default();
    if status != 0 && segs.is_empty() {
        tracing::debug!(target: "p10k", %fname, status, "user segment fn failed");
    }
    let mut segs = segs;
    for s in &mut segs {
        s.name = base.to_string();
    }
    Some(segs)
}

/// Build and install PROMPT/RPROMPT. Called from `preprompt()` after
/// the `precmd` hook has run. No-op when the engine is inactive.
///
/// p10k:5815 `_p9k_set_prompt` — walks LEFT/RIGHT_PROMPT_ELEMENTS,
/// calls each segment, assembles per-line prompt strings.
pub fn preprompt_render() {
    if !engine_active() {
        return;
    }
    let render_t0 = std::time::Instant::now();
    // RAII so every early return still logs the frame cost.
    struct RenderTimer(std::time::Instant);
    impl Drop for RenderTimer {
        fn drop(&mut self) {
            let ms = self.0.elapsed().as_millis();
            if ms > 100 {
                tracing::info!(target: "p10k", ms, "slow preprompt_render");
            } else {
                tracing::debug!(target: "p10k", ms, "preprompt_render");
            }
        }
    }
    let _t = RenderTimer(render_t0);
    let left_elems = config::p9k_global_arr("LEFT_PROMPT_ELEMENTS");
    let right_elems = config::p9k_global_arr("RIGHT_PROMPT_ELEMENTS");

    // p10k:5815+ — "newline" pseudo-elements split the element list
    // into prompt lines.
    let split_lines = |elems: &[String]| -> Vec<Vec<render::Segment>> {
        let mut lines: Vec<Vec<render::Segment>> = vec![Vec::new()];
        for name in elems {
            if name == "newline" {
                lines.push(Vec::new());
                continue;
            }
            // p10k:5834/5849 — a `<seg>_joined` element runs the base
            // segment's builder but joins the previous segment's
            // group. Dispatch on the stripped base name; write the
            // ORIGINAL suffixed element name back into each Segment so
            // render's case-2 join selection sees it (render strips it
            // again for param lookups).
            let (base, joined) = render::is_joined_name(name);
            // p10k:8290-8310 — an element with
            // POWERLEVEL9K_<ELEM>_SHOW_ON_COMMAND set is registered in
            // `_p9k_show_on_command`; p10k:7697-7726 — on every widget
            // it is shown only while the edit buffer holds a matching
            // command, and `_p9k_on_expand` hides it before each prompt
            // (p10k:6733-6736). A freshly painted prompt has an empty
            // buffer, so the paint-time state is always HIDDEN; the
            // show-while-typing re-render is not ported.
            let soc = format!(
                "POWERLEVEL9K_{}_SHOW_ON_COMMAND",
                base.replace('-', "_").to_ascii_uppercase()
            );
            if crate::ported::params::getsparam(&soc).is_some()
                || crate::ported::params::getaparam(&soc).is_some()
            {
                tracing::debug!(target: "p10k", %name, "SHOW_ON_COMMAND segment hidden at prompt paint");
                continue;
            }
            // p10k:833-840 — SHOW_ON_UPGLOB: with a pattern configured
            // the segment renders only when a parent dir (cwd → ~ or /)
            // holds a matching entry.
            if !expansion::show_on_upglob(base) {
                tracing::debug!(target: "p10k", %name, "SHOW_ON_UPGLOB: no parent match — hidden");
                continue;
            }
            let built = segments_core::build_segment(base)
                .or_else(|| segments_env::build_segment(base))
                .or_else(|| segments_sys::build_segment(base))
                .or_else(|| segments_extra::build_segment(base))
                // Beyond the p10k spec: powerline-catalog segments
                // (weather/uptime/now_playing/network_load/hg/svn/bzr/
                // fossil) and the zshrs-native introspection family
                // (zshrs_daemon/zshrs_workers/zshrs_jit/zshrs_cache/
                // zshrs_history/stryke).
                .or_else(|| segments_powerline::build_segment(base))
                .or_else(|| segments_zshrs::build_segment(base))
                // p10k:8600+ user-defined segments: a shell function
                // `prompt_<name>` emits content by calling `p10k
                // segment -t … -f …` (routed through the
                // zshrs-p10k-api bridge into USER_SEGMENT_SINK).
                .or_else(|| run_user_segment_fn(base));
            match built {
                Some(mut segs) => {
                    if joined {
                        for s in &mut segs {
                            s.name = name.clone();
                        }
                    }
                    lines.last_mut().expect("lines never empty").extend(segs);
                }
                None => {
                    tracing::debug!(target: "p10k", %name, "segment not implemented — skipped")
                }
            }
        }
        lines
    };

    let left_lines = split_lines(&left_elems);
    let right_lines = split_lines(&right_elems);
    let (prompt, rprompt) = render::render_prompt(&left_lines, &right_lines);

    crate::ported::params::setsparam("PROMPT", &prompt);
    crate::ported::params::setsparam("RPROMPT", &rprompt);
}
