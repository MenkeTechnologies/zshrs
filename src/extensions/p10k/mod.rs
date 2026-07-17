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

pub mod api;
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
pub mod wizard;

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

/// Engine on/off. Flipped by the theme-source intercept; never reset
/// (a session that loaded p10k keeps the native engine for life,
/// matching the zsh theme's own irreversibility).
static ENGINE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// wizard:1620 `__p9k_root_dir` — the p10k install root (parent of
/// `powerlevel10k.zsh-theme`), captured at theme-source intercept so
/// `p10k configure` can read the `config/p10k-*.zsh` base templates.
pub static P10K_ROOT_DIR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The p10k install root if known (theme was sourced this session).
pub fn p10k_root_dir() -> Option<String> {
    P10K_ROOT_DIR.lock().unwrap().clone()
}

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
    // wizard:1620 — `$__p9k_root_dir/config/p10k-*.zsh`: the config
    // wizard reads its base templates relative to the p10k install
    // root. The theme entry point IS `<root>/powerlevel10k.zsh-theme`,
    // so the root is the sourced file's parent dir.
    if let Some(dir) = path.rsplit_once('/').map(|(d, _)| d.to_string()) {
        *P10K_ROOT_DIR.lock().unwrap() = Some(dir);
    }
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

/// Native builtin port of the `p10k()` shell function dispatcher
/// (p10k:8983-9156). Faithful to the case structure and exit codes;
/// see api.rs for the usage strings and `display` state.
fn p10k_api(args: &[String]) -> i32 {
    // p10k:8984 — `[[ $# != 1 || $1 != finalize ]] || { … finalize; return 0 }`.
    if args.len() == 1 && args[0] == "finalize" {
        return 0; // no instant prompt — finalize is a no-op (endgame)
    }
    // p10k:8988-8991 — `if (( !ARGC )); then print usage >&2; return 1`.
    let Some(cmd) = args.first().map(String::as_str) else {
        api::print_usage(api::USAGE, true);
        return 1;
    };
    match cmd {
        // p10k:8994-8035 — `segment`.
        "segment" => p10k_segment(&args[1..]),
        // p10k:9036-9109 — `display part-pattern=state-list…` / -a / -r.
        "display" => p10k_display(&args[1..]),
        // p10k:9110-9118 — `configure`: launch the native wizard port.
        "configure" => {
            if args.len() > 1 {
                api::print_usage(api::CONFIGURE_USAGE, true); // p10k:9112
                return 1;
            }
            wizard::run(false)
        }
        // p10k:9119-9126 — `reload`.
        "reload" => {
            if args.len() > 1 {
                api::print_usage(api::RELOAD_USAGE, true); // p10k:9121
                return 1;
            }
            api::FORCE_REINIT.store(true, Ordering::Relaxed); // p10k:9125
            0
        }
        // p10k:9127-9139 — `help [command]`.
        "help" => {
            let sub = args.get(1).map(String::as_str);
            // p10k:9129-9137 — known sub → its usage rc0; bare `help` →
            // top usage rc0; unknown sub → top usage to stderr rc1.
            let known = matches!(
                sub,
                None | Some("segment")
                    | Some("display")
                    | Some("configure")
                    | Some("reload")
                    | Some("finalize")
                    | Some("help")
            );
            if known || args.len() == 1 {
                api::print_usage(api::help_usage(sub), false);
                0
            } else {
                api::print_usage(api::USAGE, true);
                1
            }
        }
        // p10k:9140-9143 — `finalize` with args is an error.
        "finalize" => {
            api::print_usage(api::FINALIZE_USAGE, true);
            1
        }
        // p10k:9144-9150 — `clear-instant-prompt`: no instant prompt.
        "clear-instant-prompt" => 0,
        // p10k:9151-9153 — unknown command.
        _ => {
            api::print_usage(api::USAGE, true);
            1
        }
    }
}

/// p10k:8994-9035 — `p10k segment` getopts `:s:b:f:i:c:t:reh` (+ the
/// `{+|-}r/e` GNU-style toggles). Emits into the active USER_SEGMENT_SINK.
fn p10k_segment(rest: &[String]) -> i32 {
    let mut state: Option<String> = None;
    let mut bg = String::new(); // p10k:8999 `bg=0` default → "0" (black)
    let mut bg_set = false;
    let mut fg = String::new();
    let mut icon = String::new();
    let mut cond = true; // p10k:9006 `-c ${OPTARG:-'${:-}'}` default true
    let mut text = String::new();
    let mut refr = false; // p10k:8999 `ref=0` — icon is symbolic by default
    let mut i = 0;
    let mut positional_seen = false;
    // p10k getopts consumes the option-argument as the NEXT word.
    let arg_at = |idx: usize| rest.get(idx + 1).cloned().unwrap_or_default();
    while i < rest.len() {
        match rest[i].as_str() {
            "-s" => {
                state = Some(arg_at(i));
                i += 1;
            }
            "-b" => {
                bg = arg_at(i);
                bg_set = true;
                i += 1;
            }
            "-f" => {
                fg = arg_at(i);
                i += 1;
            }
            "-i" => {
                icon = arg_at(i);
                i += 1;
            }
            // p10k:9006 — `-c cond`: empty after expansion → hidden.
            "-c" => {
                cond = !arg_at(i).is_empty();
                i += 1;
            }
            "-t" => {
                text = arg_at(i);
                i += 1;
            }
            // p10k:9008/9010 — `-r`/`+r`: icon is resolved-literal vs
            // symbolic ref. (Native segments carry a resolved glyph, so
            // this only records intent; both render the glyph as-is.)
            "-r" => refr = true,
            "+r" => refr = false,
            // p10k:9009/9011 — `-e`/`+e`: expand text. Native content is
            // already prompt-escaped; accepted, no separate expansion.
            "-e" | "+e" => {}
            // p10k:9012 — `-h` → usage, return 0.
            "-h" => {
                api::print_usage(api::SEGMENT_USAGE, false);
                return 0;
            }
            // p10k:9013 — unknown flag → usage to stderr, return 1.
            s if s.starts_with('-') || s.starts_with('+') => {
                api::print_usage(api::SEGMENT_USAGE, true);
                return 1;
            }
            // p10k:9016-9019 — a positional argument is an error.
            _ => {
                positional_seen = true;
            }
        }
        i += 1;
    }
    if positional_seen {
        api::print_usage(api::SEGMENT_USAGE, true);
        return 1;
    }
    let _ = refr;
    let _ = bg_set;
    USER_SEGMENT_SINK.with(|s| {
        let mut slot = s.borrow_mut();
        match slot.as_mut() {
            Some(v) => {
                // p10k:8680 / 9016 — a false cond hides the segment.
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
                // p10k:9020-9028 — "can be called only during prompt
                // rendering" (stderr, prompt-expanded like the theme).
                api::print_usage(
                    "%1F[ERROR]%f %Bp10k segment%b: can be called only during prompt rendering.",
                    true,
                );
                1
            }
        }
    })
}

/// p10k:9036-9109 — `p10k display`: `-a` dump, `-r` reset, else a list
/// of `part-pattern=state-list` toggles, refreshing the prompt when any
/// changed.
fn p10k_display(rest: &[String]) -> i32 {
    // p10k:9037-9040 — bare `p10k display` is an error.
    if rest.is_empty() {
        api::print_usage(api::DISPLAY_USAGE, true);
        return 1;
    }
    let mut dump = false; // -a
    let mut reset = false; // -r
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "-a" => dump = true,        // p10k:9053
            "-r" => reset = true,       // p10k:9046
            "-h" => {                   // p10k:9054
                api::print_usage(api::DISPLAY_USAGE, false);
                return 0;
            }
            s if s.starts_with('-') && s.len() > 1 => {
                api::print_usage(api::DISPLAY_USAGE, true); // p10k:9055
                return 1;
            }
            _ => break, // first non-option → start of the pattern list
        }
        i += 1;
    }
    let operands: Vec<&str> = rest[i..].iter().map(String::as_str).collect();

    if dump {
        // p10k:9058-9070 — populate `reply` with (name state) pairs.
        let pats = if operands.is_empty() { vec!["*"] } else { operands };
        let pairs = api::display_dump(&pats);
        crate::ported::exec::set_array("reply", pairs);
        if reset {
            preprompt_render(); // p10k:9067-9069 reset
        }
        return 0;
    }
    if reset && operands.is_empty() {
        // p10k:9046-9051 + 9106-9108 — bare `-r` redisplays.
        api::display_reset();
        preprompt_render();
        return 0;
    }
    // p10k:9074-9105 — apply each `pattern=state-list` toggle.
    let mut changed = false;
    for op in operands {
        let Some((pat, list)) = op.split_once('=') else {
            api::print_usage(api::DISPLAY_USAGE, true);
            return 1;
        };
        let states: Vec<&str> = list.split(',').filter(|s| !s.is_empty()).collect();
        if api::display_set(pat, &states) {
            changed = true;
        }
    }
    // p10k:9106-9108 — refresh the prompt if anything changed.
    if changed {
        preprompt_render();
    }
    0
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
            // p10k:9090-9097 — `p10k display '<part>'=hide` toggle.
            if api::is_hidden(base) {
                tracing::debug!(target: "p10k", %name, "hidden by p10k display toggle");
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
