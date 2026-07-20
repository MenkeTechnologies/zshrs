//! p10k transient prompt — port of the `POWERLEVEL9K_TRANSIENT_PROMPT`
//! machinery: the option declaration (p10k.zsh:7214-7215), the
//! `_p9k_transient_prompt` string construction (p10k.zsh:8313-8335)
//! and the condense decision inside `_p9k_on_widget_zle-line-finish`
//! (p10k.zsh:7623-7658).
//!
//! This file is PURE config + render — it wires no hook.
//!
//! # Integration contract (for the accept-line / zle-line-finish site)
//!
//! What zsh p10k does at line finish (p10k:7634-7653): when a transient
//! prompt is configured and the condense condition holds, it runs
//! `RPROMPT= PROMPT=$_p9k_transient_prompt _p9k_reset_prompt` — i.e.
//! `zle .reset-prompt` repaints the WHOLE just-accepted prompt block
//! (every PROMPT line plus the right prompt, not just the last line)
//! with the one-line transient string; the accepted command text stays
//! on that line and command output begins below it.
//!
//! The native site must therefore, at accept time and BEFORE any
//! command output:
//!
//! 1. `let Some(mode) = transient_enabled() else { return }` —
//!    engine active and mode != off (p10k:7634 `-n $_p9k_transient_prompt`).
//! 2. `should_condense(&recorded_pwd, &cwd, mode)` where `recorded_pwd`
//!    is the integration's own `_p9k__last_prompt_pwd` slot
//!    (p10k:7180, starts empty → first accept never condenses under
//!    same-dir). When it returns FALSE under SameDir, store `cwd` into
//!    the slot (p10k:7639 `_p9k__last_prompt_pwd=$_p9k__cwd`); when it
//!    returns TRUE the slot is left untouched (p10k:7636-7637 only set
//!    `optimized=1`, no pwd write).
//! 3. On TRUE: `let (left, right) = render_transient();` — `right` is
//!    always "" (p10k:7653 `RPROMPT=`). Repaint in place: the previous
//!    live refresh left the cursor on the prompt's LAST physical row
//!    and recorded the block height in
//!    `crate::ported::zle::zle_refresh::LAST_PAINT_ROWS`
//!    (zle_refresh.rs:4998, stored at zle_refresh.rs:1247). Move the
//!    cursor up `LAST_PAINT_ROWS - 1` rows, `\r`, clear to end of
//!    screen (`\x1b[J`), then print the prompt-expanded `left` followed
//!    by the accepted buffer — exactly the repaint-anchor dance the
//!    live refresh does at zle_refresh.rs:1242-1247, with `left` in
//!    place of PROMPT.
//! 4. Reentrancy: p10k:7625 guards with `_p9k__line_finished` so a
//!    second line-finish for the same line is a no-op — the integration
//!    needs the same one-shot latch per accepted line (send-break also
//!    funnels here, p10k:7660-7662).

use crate::extensions::p10k::config::{p9k_global, p9k_param};
use crate::ported::params::getsparam;
use std::path::Path;

/// p10k:7214-7215 — `_p9k_declare -s POWERLEVEL9K_TRANSIENT_PROMPT off`
/// validated against `(off|always|same-dir)`; anything else falls back
/// to `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransientMode {
    /// `always` — condense every accepted prompt (p10k:7635 first arm).
    Always,
    /// `same-dir` — condense only when the accept happens in the same
    /// directory as the recorded previous prompt (p10k:7635 second arm).
    SameDir,
}

/// p10k:7215 — `[[ $_POWERLEVEL9K_TRANSIENT_PROMPT == (off|always|same-dir) ]]
/// || _POWERLEVEL9K_TRANSIENT_PROMPT=off`: exact-string match, invalid
/// and unset both mean off. Pure so the parse is testable without the
/// global engine flag.
fn parse_mode(value: &str) -> Option<TransientMode> {
    match value {
        "always" => Some(TransientMode::Always),
        "same-dir" => Some(TransientMode::SameDir),
        _ => None, // "off" and everything invalid (p10k:7215)
    }
}

/// Read `POWERLEVEL9K_TRANSIENT_PROMPT` from the live paramtab.
/// `None` = transient prompt disabled (mode off / invalid / engine not
/// active — with the engine off `_p9k_transient_prompt` was never
/// built, matching p10k:7634's `-n $_p9k_transient_prompt` gate).
pub fn transient_enabled() -> Option<TransientMode> {
    if !crate::p10k::engine_active() {
        return None;
    }
    parse_mode(&p9k_global("TRANSIENT_PROMPT", "off"))
}

/// Condense decision of `_p9k_on_widget_zle-line-finish`
/// (p10k:7635 — `[[ $_POWERLEVEL9K_TRANSIENT_PROMPT == always ||
/// $_p9k__cwd == $_p9k__last_prompt_pwd ]]`).
///
/// `pwd_at_accept` is the caller's `_p9k__last_prompt_pwd` slot (the
/// cwd recorded at the previous NON-condensed accept; p10k:7180 —
/// starts empty, so pass e.g. `Path::new("")` before the first accept),
/// `pwd_now` is `_p9k__cwd` at this accept. On a `false` result under
/// [`TransientMode::SameDir`] the caller must record `pwd_now` into its
/// slot (p10k:7639); on `true` the slot stays untouched (p10k:7636-7637).
pub fn should_condense(pwd_at_accept: &Path, pwd_now: &Path, mode: TransientMode) -> bool {
    match mode {
        TransientMode::Always => true,
        // p10k:7635 — plain string equality of the two pwd values.
        TransientMode::SameDir => pwd_at_accept == pwd_now,
    }
}

/// p10k:532-541 (`_p9k_translate_color`) — minimal local translation:
/// decimal codes are zero-padded to 3 digits (p10k:534
/// `${(l.3..0.)1}`), `#hex` is lowercased (p10k:536); everything else
/// passes through unchanged. The full `__p9k_colors` name table lives
/// privately in render.rs; prompt_char foregrounds are numeric in
/// every stock config (76/196), and the zsh prompt expander resolves
/// basic color NAMES in `%F{...}` natively, so the table is not
/// duplicated here.
fn translate_color_min(c: &str) -> String {
    if !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()) {
        format!("{c:0>3}")
    } else if c.len() > 1 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        c.to_ascii_lowercase()
    } else {
        c.to_string()
    }
}

/// One `%(?...)` branch of the transient string (p10k:8316-8328): the
/// prompt_char foreground for the given state + the glyph.
///
/// p10k:8316/8323 — `_p9k_color prompt_prompt_char_<STATE> FOREGROUND
/// 76|196` then `_p9k_foreground` (`%F{c}`, or `%f` when the resolved
/// color is empty — p10k:590-595).
///
/// p10k:8319/8326 — content is `${${P9K_CONTENT::="❯"}+}` (assign, emit
/// nothing) followed by the CONTENT_EXPANSION param (default
/// `'${P9K_CONTENT}'`, p10k:8320/8327) wrapped in `${:-"..."}`. Same
/// phase-1 CONTENT_EXPANSION policy as render.rs: default passthrough
/// → the glyph, empty → hide; any other custom zsh expansion is logged
/// and rendered as the default glyph.
fn transient_char(state: &str, default_fg: &str) -> String {
    // p10k:3313 — the prompt_char segment's style name is
    // `prompt_prompt_char` (segment "prompt_char" under the
    // `prompt_`-prefixing probe), so the probe chain here hits
    // POWERLEVEL9K_PROMPT_CHAR_<STATE>_FOREGROUND etc.
    let fg = translate_color_min(&p9k_param(
        "prompt_char",
        Some(state),
        "FOREGROUND",
        default_fg,
    ));
    // p10k:590-595 — `_p9k_foreground`: `%F{c}` or `%f` for default.
    let fg_seq = if fg.is_empty() {
        "%f".to_string()
    } else {
        format!("%F{{{fg}}}")
    };
    let ce = p9k_param(
        "prompt_char",
        Some(state),
        "CONTENT_EXPANSION",
        "${P9K_CONTENT}",
    );
    let glyph = match ce.as_str() {
        "" => "", // empty expansion hides the char (render.rs policy)
        "${P9K_CONTENT}" | "$P9K_CONTENT" => "\u{276F}", // ❯ (p10k:8319)
        other => {
            tracing::debug!(target: "p10k", state, expansion = %other,
                "custom prompt_char CONTENT_EXPANSION not evaluated in transient prompt — default glyph used");
            "\u{276F}"
        }
    };
    format!("{fg_seq}{glyph}")
}

/// Build the condensed prompt pair `(PROMPT, RPROMPT)` — port of the
/// `_p9k_transient_prompt` construction (p10k:8313-8335) evaluated
/// eagerly.
///
/// The zsh string is
/// `%b%k%s%u%(?<sep><OK branch><sep><ERROR branch>)%b%k%f%s%u ` — a
/// runtime `%(?...)` ternary. At the accept-time repaint `$?` is still
/// the status of the command BEFORE the accepted line, which is exactly
/// [`crate::p10k::last_status`] (mod.rs snapshots it pre-precmd), so
/// the ternary collapses at render time — same eager philosophy as
/// render.rs. Both branches hardcode the VIINS states (p10k:8316
/// `prompt_prompt_char_OK_VIINS`, p10k:8323
/// `prompt_prompt_char_ERROR_VIINS`) — the transient char never shows
/// vicmd/visual shapes.
///
/// RPROMPT is always empty: p10k:7653 —
/// `RPROMPT= PROMPT=$_p9k_transient_prompt _p9k_reset_prompt`.
pub fn render_transient() -> (String, String) {
    // p10k:8315 — leading attribute reset `%b%k%s%u`.
    let mut left = String::from("%b%k%s%u");
    // p10k:8315-8329 — `%(?` OK branch / ERROR branch `)`, collapsed
    // on the snapshotted status (OK default fg 76, ERROR 196).
    if crate::p10k::last_status() == 0 {
        left.push_str(&transient_char("OK_VIINS", "76")); // p10k:8316-8321
    } else {
        left.push_str(&transient_char("ERROR_VIINS", "196")); // p10k:8323-8328
    }
    // p10k:8329 — trailing reset + the separating space.
    left.push_str("%b%k%f%s%u ");
    // p10k:7217-7219 — TERM_SHELL_INTEGRATION: `_p9k_declare -b ... 0`,
    // forced on when $ITERM_SHELL_INTEGRATION_INSTALLED == Yes.
    let shell_integration = p9k_global("TERM_SHELL_INTEGRATION", "") == "true"
        || getsparam("ITERM_SHELL_INTEGRATION_INSTALLED").as_deref() == Some("Yes");
    if shell_integration {
        // p10k:8330-8331 — wrap in OSC 133 prompt-start/end marks. The
        // z4h/tmux DCS variant (p10k:8332-8334) is z4h-only, unported.
        left = format!("%{{\u{1b}]133;A\u{7}%}}{left}%{{\u{1b}]133;B\u{7}%}}");
    }
    (left, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// p10k:7215 — only the exact strings `always` / `same-dir` enable
    /// a mode; `off`, unset-equivalent empty, and garbage are all off.
    #[test]
    fn mode_parsing_exact_strings_only() {
        assert_eq!(parse_mode("always"), Some(TransientMode::Always));
        assert_eq!(parse_mode("same-dir"), Some(TransientMode::SameDir));
        assert_eq!(parse_mode("off"), None);
        assert_eq!(parse_mode(""), None);
        assert_eq!(parse_mode("Always"), None); // case-sensitive glob
        assert_eq!(parse_mode("same_dir"), None);
        assert_eq!(parse_mode("true"), None);
    }

    /// p10k:7635 — `always` condenses regardless of directories.
    #[test]
    fn always_condenses_everywhere() {
        assert!(should_condense(
            Path::new("/a"),
            Path::new("/b"),
            TransientMode::Always
        ));
        assert!(should_condense(
            Path::new(""),
            Path::new("/b"),
            TransientMode::Always
        ));
    }

    /// p10k:7635 — `same-dir` condenses only on pwd equality; the
    /// empty initial slot (p10k:7180) never matches a real cwd, so the
    /// first accept is not condensed.
    #[test]
    fn same_dir_requires_pwd_match() {
        assert!(should_condense(
            Path::new("/home/u/proj"),
            Path::new("/home/u/proj"),
            TransientMode::SameDir
        ));
        assert!(!should_condense(
            Path::new("/home/u/proj"),
            Path::new("/home/u/other"),
            TransientMode::SameDir
        ));
        // Initial empty slot (p10k:7180 typeset -g, no value).
        assert!(!should_condense(
            Path::new(""),
            Path::new("/home/u/proj"),
            TransientMode::SameDir
        ));
    }

    /// p10k:534/536 — decimal pad + hex lowercase; names pass through.
    #[test]
    fn translate_color_min_forms() {
        assert_eq!(translate_color_min("76"), "076");
        assert_eq!(translate_color_min("196"), "196");
        assert_eq!(translate_color_min("#FFAA00"), "#ffaa00");
        assert_eq!(translate_color_min("green"), "green");
        assert_eq!(translate_color_min(""), "");
    }
}

// !!! PLACEHOLDER STUB — the concurrent p10k session owns this file and is
// writing the real body (zle_main.rs:1273 already calls it). `None` =
// transient prompt disabled this accept. Replace wholesale. !!!
/// Returns the (PROMPT, RPROMPT) template pair to repaint the accepted
/// line with, when the transient prompt is enabled.
pub fn transient_swap_for_accept() -> Option<(String, String)> {
    None
}
