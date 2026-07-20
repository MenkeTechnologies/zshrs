//! Port of the `p10k()` shell FUNCTION (internal/p10k.zsh:8983-9156)
//! as a native builtin dispatcher, plus the usage strings
//! (p10k:8824-8935) and the `p10k display` visibility-override state.
//!
//! The zsh theme defines `p10k` as a function whose single argument
//! selects a subcommand: `segment` (emit a user segment mid-render),
//! `display` (show/hide/toggle prompt parts), `reload`, `configure`,
//! `help`, `finalize`, `clear-instant-prompt`. With the theme never
//! executing, calls to `p10k <cmd>` from .zshrc / zpwr land here.
//!
//! `p10k configure` (the 2153-line `internal/wizard.zsh`) is out of
//! native-engine scope — absorbed with a log, matching the endgame
//! rule "accept deprecated/unsupported calls silently for compat".

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// p10k:9124 `_p9k__force_must_init=1` — `p10k reload` sets it so the
/// theme re-reads config. The native engine reads the paramtab live on
/// every render, so this is observational only (exposed for tests /
/// future daemon cache-invalidation).
pub static FORCE_REINIT: AtomicBool = AtomicBool::new(false);

/// p10k:9036-9109 `p10k display` state. Maps a prompt-part pattern
/// (segment base name, or the `<i>/<side>/<name>` slot forms the theme
/// uses) to its current visibility. Absent = shown (the default).
/// `split_lines` (mod.rs) consults `is_hidden` before building each
/// element; `p10k display 'X'=hide` / `=show` / `=hide,show` (toggle)
/// flip it.
static DISPLAY_STATE: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

/// True when the part named `seg` (a segment base name) is currently
/// hidden by a `p10k display` toggle. Matched against the stored keys
/// both directly and via the `*/<name>` / `<i>/<side>/<name>` tails the
/// theme's slot names use (p10k:9095-9097).
pub fn is_hidden(seg: &str) -> bool {
    let g = DISPLAY_STATE.lock().unwrap();
    let Some(map) = g.as_ref() else { return false };
    if map.is_empty() {
        return false;
    }
    map.iter()
        .any(|(k, &hidden)| hidden && part_matches(k, seg))
}

/// A stored display key matches segment `seg` when it is the bare name,
/// or ends in `/<seg>` (the theme's `<i>/<side>/<name>` and `*/<name>`
/// slot forms, p10k:9091-9097).
fn part_matches(key: &str, seg: &str) -> bool {
    key == seg
        || key.rsplit('/').next() == Some(seg)
        || (key.ends_with(seg) && key[..key.len() - seg.len()].ends_with('/'))
}

/// p10k:9036-9109 — apply one `part-pattern=state-list` toggle. A
/// single-element list sets the state directly; a multi-element list
/// (`hide,show`) toggles through the cycle from the current state
/// (p10k:9083-9088). Returns whether anything changed (drives the
/// reset-prompt at p10k:9106).
pub fn display_set(pattern: &str, states: &[&str]) -> bool {
    if states.is_empty() {
        return false;
    }
    let mut g = DISPLAY_STATE.lock().unwrap();
    let map = g.get_or_insert_with(HashMap::new);
    // p10k only tracks show/hide as the meaningful terminal states.
    let cur_hidden = map.get(pattern).copied().unwrap_or(false);
    let cur = if cur_hidden { "hide" } else { "show" };
    let new = if states.len() == 1 {
        states[0]
    } else {
        // p10k:9087 — next entry after the current one, wrapping.
        match states.iter().position(|s| *s == cur) {
            Some(i) => states[(i + 1) % states.len()],
            None => states[0],
        }
    };
    let new_hidden = new == "hide";
    if new_hidden == cur_hidden && map.contains_key(pattern) {
        return false; // p10k:9084/9088 — no change, `continue`.
    }
    map.insert(pattern.to_string(), new_hidden);
    true
}

/// p10k:9058-9065 `p10k display -a` — the visibility of each part
/// matching `patterns` (default `*`), as `(name state)` pairs for the
/// `reply` array.
pub fn display_dump(patterns: &[&str]) -> Vec<String> {
    let g = DISPLAY_STATE.lock().unwrap();
    let Some(map) = g.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, &hidden) in map.iter() {
        let show = patterns.is_empty()
            || patterns
                .iter()
                .any(|p| *p == "*" || part_matches(k, p) || *p == k.as_str());
        if show {
            out.push(k.clone());
            out.push(if hidden { "hide".into() } else { "show".into() });
        }
    }
    out
}

/// p10k:9046-9051 `p10k display -r` — clear every toggle so the next
/// render shows all parts.
pub fn display_reset() {
    if let Some(map) = DISPLAY_STATE.lock().unwrap().as_mut() {
        map.clear();
    }
}

// --------------------------------------------------------------------
// Usage strings — verbatim from p10k:8824-8935 (leading `%2F`/`%B`
// prompt escapes intact; the dispatcher prints them through
// `promptexpand`, exactly as the theme's `print -rP` does).
// --------------------------------------------------------------------

/// p10k:8824 `__p9k_p10k_usage`.
pub const USAGE: &str = "Usage: %2Fp10k%f %Bcommand%b [options]

Commands:

  %Bconfigure%b  run interactive configuration wizard
  %Breload%b     reload configuration
  %Bsegment%b    print a user-defined prompt segment
  %Bdisplay%b    show, hide or toggle prompt parts
  %Bhelp%b       print this help message

Print help for a specific command:

  %2Fp10k%f %Bhelp%b command";

/// p10k:8838 `__p9k_p10k_segment_usage` (options block; the long
/// worked example is elided — `p10k help segment` still names it).
pub const SEGMENT_USAGE: &str =
    "Usage: %2Fp10k%f %Bsegment%b [-h] [{+|-}re] [-s state] [-b bg] [-f fg] [-i icon] [-c cond] [-t text]

Print a user-defined prompt segment. Can be called only during prompt rendering.";

/// p10k:8908 `__p9k_p10k_configure_usage`.
pub const CONFIGURE_USAGE: &str = "Usage: %2Fp10k%f %Bconfigure%b

Run interactive configuration wizard.";

/// p10k:8911 `__p9k_p10k_reload_usage`.
pub const RELOAD_USAGE: &str = "Usage: %2Fp10k%f %Breload%b

Reload configuration.";

/// p10k:8915 `__p9k_p10k_finalize_usage`.
pub const FINALIZE_USAGE: &str = "Usage: %2Fp10k%f %Bfinalize%b

Perform the final stage of initialization. Must be called at the very end of zshrc.";

/// p10k:8919 `__p9k_p10k_display_usage` (synopsis; the parts list is
/// elided).
pub const DISPLAY_USAGE: &str = "Usage: %2Fp10k%f %Bdisplay%b part-pattern=state-list...

  Show, hide or toggle prompt parts. If called from zle, the current
  prompt is refreshed.

Usage: %2Fp10k%f %Bdisplay%b -a [part-pattern]...

  Populate array `reply` with states of prompt parts matching the patterns.

Usage: %2Fp10k%f %Bdisplay%b -r

  Redisplay prompt.";

/// p10k:9128 — `local var=__p9k_p10k_$2_usage; print -rP ${(P)var}`:
/// the usage string for `p10k help <sub>`, or the top usage.
pub fn help_usage(sub: Option<&str>) -> &'static str {
    match sub {
        None => USAGE,
        Some("segment") => SEGMENT_USAGE,
        Some("configure") => CONFIGURE_USAGE,
        Some("reload") => RELOAD_USAGE,
        Some("finalize") => FINALIZE_USAGE,
        Some("display") => DISPLAY_USAGE,
        Some("help") => USAGE,
        _ => USAGE,
    }
}

/// Print a usage string the way `print -rP` does — prompt-expanded to
/// the caller's fd. `to_stderr` mirrors the theme's `>&2` on the error
/// paths (p10k:8989/9013/…).
pub fn print_usage(s: &str, to_stderr: bool) {
    let (expanded, _, _) = crate::ported::prompt::promptexpand(s, 0, None);
    if to_stderr {
        eprintln!("{expanded}");
    } else {
        println!("{expanded}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        display_reset();
    }

    #[test]
    fn display_hide_show_toggle() {
        reset();
        assert!(!is_hidden("dir"));
        assert!(display_set("dir", &["hide"]));
        assert!(is_hidden("dir"));
        // idempotent set → no change.
        assert!(!display_set("dir", &["hide"]));
        assert!(display_set("dir", &["show"]));
        assert!(!is_hidden("dir"));
        // toggle list cycles show→hide.
        assert!(display_set("dir", &["hide", "show"]));
        assert!(is_hidden("dir"));
        reset();
        assert!(!is_hidden("dir"));
    }

    #[test]
    fn slot_name_matches_segment() {
        reset();
        // p10k slot form `1/left/dir` hides the `dir` segment.
        assert!(display_set("1/left/dir", &["hide"]));
        assert!(is_hidden("dir"));
        // but not an unrelated segment.
        assert!(!is_hidden("time"));
        reset();
    }

    #[test]
    fn dump_reports_pairs() {
        reset();
        display_set("dir", &["hide"]);
        let d = display_dump(&["*"]);
        assert_eq!(d, vec!["dir".to_string(), "hide".to_string()]);
        reset();
    }
}
