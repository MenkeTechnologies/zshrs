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
pub mod git;
pub mod icons;
pub mod render;
pub mod segments_core;
pub mod segments_env;

use std::sync::atomic::{AtomicBool, Ordering};

/// Engine on/off. Flipped by the theme-source intercept; never reset
/// (a session that loaded p10k keeps the native engine for life,
/// matching the zsh theme's own irreversibility).
static ENGINE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn engine_active() -> bool {
    ENGINE_ACTIVE.load(Ordering::Relaxed)
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
        // p10k.zsh:8600+ `function p10k()` — segment/display/reload
        // API. Live paramtab reads make reload/refresh no-ops;
        // `p10k segment` (used inside custom segments) is handled by
        // the segment runner, not here.
        "p10k" | "p10k-instant-prompt-finalize" => {
            tracing::debug!(target: "p10k", ?args, "p10k command call absorbed by native engine");
            Some(0)
        }
        _ => None,
    }
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
            let built = segments_core::build_segment(name)
                .or_else(|| segments_env::build_segment(name));
            match built {
                Some(segs) => lines.last_mut().expect("lines never empty").extend(segs),
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
