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
        assert!(!names.contains(&"EXTENDED_GLOB"), "set option leaked through filter");
        assert!(!names.contains(&"PIPE_FAIL"));
    }
}
