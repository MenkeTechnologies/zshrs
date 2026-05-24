//! Port of `_options_unset` — complete currently unset options.
//!
//! Local shell reference: `compsys/functions/Zsh/Type/_options_unset`
//! (system copy `/opt/homebrew/share/zsh/functions/_options_unset`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  5  list=( ${(@k)options[(R)off]} )
//!  7  _wanted options expl 'unset option' compadd "$@" -a list
//! ```
//!
//! Faithful Rust port: mirrors `_options_set` but inverts the
//! filter (`!is_set` instead of `is_set`).

use crate::compcore::CompletionState;

use super::_options::_options;

/// _options_unset - Complete currently unset options
pub fn _options_unset(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let unset_opts: Vec<(&str, bool)> = shell_options
        .iter()
        .filter(|(_, is_set)| !*is_set)
        .copied()
        .collect();
    _options(state, &unset_opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_to_unset_only() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![
            ("EXTENDED_GLOB", true),
            ("NULL_GLOB", false),
            ("PIPE_FAIL", true),
            ("NO_BANG_HIST", false),
        ];
        let ok = _options_unset(&mut state, &opts);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"NULL_GLOB"));
        assert!(names.contains(&"NO_BANG_HIST"));
        assert!(!names.contains(&"EXTENDED_GLOB"));
        assert!(!names.contains(&"PIPE_FAIL"));
    }

    #[test]
    fn returns_false_when_no_unset_options() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![("A", true), ("B", true)];
        assert!(!_options_unset(&mut state, &opts));
    }

    #[test]
    fn empty_options_list_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_options_unset(&mut state, &[]));
    }

    #[test]
    fn prefix_filters_combined_with_set_filter() {
        let mut state = CompletionState::new();
        state.params.prefix = "NO".into();
        let opts: Vec<(&str, bool)> = vec![
            ("NO_BANG_HIST", false),
            ("NO_BEEP", true), // set, should be excluded
            ("NULL_GLOB", false),
        ];
        let ok = _options_unset(&mut state, &opts);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"NO_BANG_HIST"));
        assert!(!names.contains(&"NO_BEEP"), "set option must not appear");
        assert!(!names.contains(&"NULL_GLOB"), "off-prefix must not appear");
    }
}
