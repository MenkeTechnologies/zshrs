//! Port of `_completers` (zsh Completion/Base/Utility/_completers).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_completers`.
//!
//! 14-line shell function. Honors:
//!   - `-p` flag → prepend `_` to each emitted name (form usable in
//!     `compdef -K`)
//!   - `prefix-hidden` zstyle → set `disp` to the bare name while
//!     `str_` carries `_xxx`, mirroring shell's `-d list` alias

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// Canonical 11-completer list emitted by `_completers` (shell:7-8).
/// Mirrors the upstream `list=(…)` literal verbatim.
pub const CANONICAL_COMPLETER_NAMES: &[&str] = &[
    "complete",
    "approximate",
    "correct",
    "match",
    "expand",
    "list",
    "menu",
    "oldlist",
    "ignored",
    "prefix",
    "history",
];

pub fn completers(state: &mut CompletionState, with_underscore_prefix: bool) -> bool {
    let prefix = state.params.prefix.clone();
    let curcontext = "::completers";

    // shell:11-12: zstyle -t :completion:${curcontext}:completers prefix-hidden
    let prefix_hidden = state
        .styles
        .lookup_bool(
            &format!(":completion:{}:completers", curcontext),
            "prefix-hidden",
        )
        .unwrap_or(false);

    let us = if with_underscore_prefix { "_" } else { "" };

    state.begin_group("completers", true);
    for &bare in CANONICAL_COMPLETER_NAMES {
        let inserted = format!("{}{}", us, bare);
        if !inserted.starts_with(&prefix) {
            continue;
        }
        let mut c = Completion::new(inserted);
        // `prefix-hidden`: shell uses `-d list` so listing shows the
        // BARE name (`complete`) while insertion still uses `_complete`.
        if prefix_hidden && with_underscore_prefix {
            c.disp = Some(bare.into());
        }
        state.add_match(c, Some("completers"));
    }
    state.end_group();
    state.nmatches > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_canonical_eleven_bare() {
        let mut state = CompletionState::new();
        completers(&mut state, false);
        let mut names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "approximate",
                "complete",
                "correct",
                "expand",
                "history",
                "ignored",
                "list",
                "match",
                "menu",
                "oldlist",
                "prefix",
            ],
        );
    }

    #[test]
    fn with_p_flag_adds_underscore_prefix() {
        let mut state = CompletionState::new();
        completers(&mut state, true);
        let names: std::collections::HashSet<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names.len(), 11);
        assert!(names.contains("_complete"));
        assert!(names.contains("_approximate"));
        assert!(names.contains("_history"));
        assert!(!names.contains("complete"));
    }

    #[test]
    fn prefix_hidden_shows_bare_in_disp() {
        let mut state = CompletionState::new();
        state.styles.set(
            ":completion:::completers:completers",
            "prefix-hidden",
            vec!["true".into()],
            false,
        );
        completers(&mut state, true);
        let c = state.groups[0]
            .matches
            .iter()
            .find(|m| m.str_ == "_complete")
            .expect("_complete present");
        assert_eq!(c.disp.as_deref(), Some("complete"));
    }
}
