//! p10k transient prompt — port of the `POWERLEVEL9K_TRANSIENT_PROMPT`
//! machinery: the option declaration (p10k.zsh:7214-7215), the
//! `_p9k_transient_prompt` string construction (p10k.zsh:8313-8335)
//! and the condense decision inside `_p9k_on_widget_zle-line-finish`
//! (p10k.zsh:7623-7658).
//!
//! The accept-time hook is [`transient_swap_for_accept`], called from
//! zle_main.rs right before the final `trashzle()` repaint. It mirrors
//! `_p9k_on_widget_zle-line-finish` (p10k:7897-7933):
//!
//! 1. the user's `p10k-on-post-prompt` runs and the line is marked
//!    finished (p10k:7901/7933);
//! 2. with a transient prompt configured ([`transient_enabled`],
//!    p10k:7909), [`should_condense`] decides against the recorded
//!    `_p9k__last_prompt_pwd` slot (p10k:7404 — empty at start, so the
//!    first accept never condenses under `same-dir`; written on a
//!    non-condensing accept, left alone on a condensing one);
//! 3. on condense the pair from [`render_transient`] replaces the
//!    prompt buffers (`RPROMPT` always empty, p10k:7926);
//! 4. otherwise, when a segment renders differently once the line is
//!    finished (TIME_UPDATE_ON_COMMAND), the prompt is re-rendered and
//!    that pair is returned.

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

/// p10k:532-541 `_p9k_translate_color` — render.rs owns the table.
fn translate_color_min(c: &str) -> String {
    crate::extensions::p10k::render::translate_color(c)
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
/// `'${P9K_CONTENT}'`, p10k:8320/8327) wrapped in `${:-"..."}`; the
/// template is evaluated by expansion.rs exactly as for the live
/// prompt_char segment.
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
    // p10k:8319/8327 — `${${P9K_CONTENT::="❯"}+}` assigns the glyph, the
    // CONTENT_EXPANSION template (default `${P9K_CONTENT}`) then expands
    // around it.
    let glyph = crate::p10k::expansion::apply_content_expansion(
        "prompt_char",
        Some(state),
        "\u{276F}",
    );
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
        // The z4h/tmux DCS variant (p10k:8332-8334) applies only inside
        // z4h, which this engine replaces.
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
        assert_eq!(translate_color_min("green"), "002");
        assert_eq!(translate_color_min(""), "");
    }
}

/// `_p9k__last_prompt_pwd` (p10k:7180) — cwd recorded at the previous
/// non-condensed accept; starts empty so the first accept never
/// condenses under `same-dir`.
static LAST_PROMPT_PWD: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Accept-time hook (`_p9k_on_widget_zle-line-finish`, p10k:7897-7933).
/// Returns the (PROMPT, RPROMPT) template pair to repaint the accepted
/// line with: the condensed transient pair, or — when the prompt holds
/// content that updates on command (time with
/// TIME_UPDATE_ON_COMMAND, `_p9k_reset_on_line_finish`) — the freshly
/// re-rendered prompt. `None` = repaint nothing. One call per accepted
/// line, which is the `_p9k__line_finished` latch (p10k:7898).
pub fn transient_swap_for_accept() -> Option<(String, String)> {
    if !crate::p10k::engine_active() {
        return None;
    }
    crate::p10k::run_post_prompt_hook(); // p10k:7901
    crate::p10k::mark_line_finished(); // p10k:7933
    if let Some(mode) = transient_enabled() {
        // p10k:7909-7914 — transient prompt configured.
        let cwd = std::path::PathBuf::from(getsparam("PWD").filter(|p| !p.is_empty()).unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        }));
        let mut slot = LAST_PROMPT_PWD.lock().unwrap();
        if should_condense(Path::new(slot.as_str()), &cwd, mode) {
            // p10k:7910-7911 + 7926 — condense; slot untouched.
            return Some(render_transient());
        }
        // p10k:7913 — `_p9k__last_prompt_pwd=$_p9k__cwd`.
        *slot = cwd.to_string_lossy().into_owned();
    }
    // p10k:7924-7925 — plain `_p9k_reset_prompt` repaint, needed only
    // when a segment renders differently once the line is finished.
    if p9k_global("TIME_UPDATE_ON_COMMAND", "") == "true" {
        crate::p10k::preprompt_render();
        return Some((
            getsparam("PROMPT").unwrap_or_default(),
            getsparam("RPROMPT").unwrap_or_default(),
        ));
    }
    None
}
