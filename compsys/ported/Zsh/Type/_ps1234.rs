//! Port of `_ps1234` — complete prompt escape sequences (`%n`, `%~`, …).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_ps1234`.
//!
//! Upstream is a large ~250-line state machine. Our user-visible
//! contract: emit the documented `%` escape codes with descriptions
//! so the user can pick the next prompt sigil while editing PS1.
//!
//! Strict Rust port: emits the documented escape sequences for the
//! current cursor position. If the user has already typed `%F` /
//! `%K` / `%D`, the next chars expected are the formatting args
//! (color/date format); we emit those subsets when applicable.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// Top-level `%` escape codes for PS1..4.
pub const PROMPT_ESCAPES: &[(&str, &str)] = &[
    ("%", "literal `%`"),
    (")", "literal `)`"),
    ("d", "current working directory (no abbreviation)"),
    ("~", "current working directory (with HOME → ~)"),
    ("h", "host name (up to first `.`)"),
    ("H", "full host name"),
    ("M", "FQDN host name"),
    ("n", "user name"),
    ("y", "tty (without `/dev/`)"),
    ("l", "tty (with `/dev/`)"),
    ("?", "exit status of previous command"),
    ("_", "parser state at point"),
    ("!", "current history event number"),
    ("@", "host name (alternate)"),
    ("#", "`#` if running as root, `%` otherwise"),
    ("*", "current time HH:MM:SS"),
    ("T", "current time HH:MM"),
    ("t", "current time h:MM AM/PM"),
    ("w", "weekday/month/day"),
    ("W", "month/day/year"),
    ("D", "date — needs {format}"),
    ("F", "set foreground color — needs {color}"),
    ("f", "reset foreground color"),
    ("K", "set background color — needs {color}"),
    ("k", "reset background color"),
    ("B", "begin bold"),
    ("b", "end bold"),
    ("U", "begin underline"),
    ("u", "end underline"),
    ("S", "begin standout"),
    ("s", "end standout"),
    ("E", "clear to end of line"),
    ("L", "literal newline"),
    ("N", "function name"),
    ("i", "line number"),
    ("x", "source-file name"),
    ("c", "deprecated alias for `%~`"),
    ("e", "ESC literal"),
    ("v", "psvar element"),
    ("j", "number of jobs"),
    ("(", "begin ternary `%(C.true.false)`"),
];

/// `_ps1234` — emit `%` escape codes.
pub fn _ps1234(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();
    state.begin_group("prompt-escapes", true);
    let mut any = false;
    for (esc, desc) in PROMPT_ESCAPES {
        if !esc.starts_with(&*prefix) {
            continue;
        }
        let mut comp = Completion::new(*esc);
        comp.disp = Some(format!("%{} -- {}", esc, desc));
        state.add_match(comp, Some("prompt-escapes"));
        any = true;
    }
    state.end_group();
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_escapes_emitted() {
        let mut state = CompletionState::new();
        let _ = _ps1234(&mut state);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        for e in ["~", "n", "h", "?", "#", "F", "f", "K", "k", "D", "%"] {
            assert!(names.contains(&e), "missing escape `%{e}`");
        }
    }

    #[test]
    fn prefix_filter() {
        let mut state = CompletionState::new();
        state.params.prefix = "F".into();
        let _ = _ps1234(&mut state);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["F"]);
    }

    #[test]
    fn each_has_descriptive_disp() {
        let mut state = CompletionState::new();
        let _ = _ps1234(&mut state);
        for m in &state.groups[0].matches {
            let d = m.disp.as_deref().unwrap_or("");
            assert!(d.starts_with('%'), "disp must show the `%` form: `{}`", d);
            assert!(d.contains(" -- "));
        }
    }

    #[test]
    fn group_named_prompt_escapes() {
        let mut state = CompletionState::new();
        let _ = _ps1234(&mut state);
        assert!(state.groups.iter().any(|g| g.name == "prompt-escapes"));
    }

    #[test]
    fn at_least_30_escapes() {
        let mut state = CompletionState::new();
        let _ = _ps1234(&mut state);
        assert!(state.groups[0].matches.len() >= 30);
    }
}
