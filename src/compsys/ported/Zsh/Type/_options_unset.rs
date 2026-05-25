//! Port of `_options_unset` from `Completion/Zsh/Type/_options_unset`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete all unset options. This relies on `_main_complete' to store the
//! sh: 4  # names of the options that were unset when it was called in the array
//! sh: 5  # `_options_unset'.
//! sh: 6
//! sh: 7  local expl
//! sh: 8
//! sh: 9  _wanted zsh-options expl 'unset zsh option' \
//! sh:10      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -a - _options_unset
//! ```
//!
//! Faithful Rust port: mirrors `_options_set` but inverts the
//! filter (`!is_set` instead of `is_set`).



use crate::compsys::compcore::CompletionState;

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

    #[test]
    fn complement_of_options_set() {
        // For any (key, is_set) entry: _options_unset and _options_set
        // should be exact complements.
        use crate::compsys::ported::_options_set::_options_set;
        let opts: Vec<(&str, bool)> = vec![
            ("A", true),
            ("B", false),
            ("C", true),
            ("D", false),
            ("E", false),
        ];
        let mut s1 = CompletionState::new();
        let mut s2 = CompletionState::new();
        let _ = _options_set(&mut s1, &opts);
        let _ = _options_unset(&mut s2, &opts);
        let set: std::collections::HashSet<String> = s1
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        let unset: std::collections::HashSet<String> = s2
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            set.is_disjoint(&unset),
            "set and unset must be disjoint; set={set:?} unset={unset:?}"
        );
        let total = set.len() + unset.len();
        assert_eq!(
            total,
            opts.len(),
            "every option must appear in exactly one bucket"
        );
    }

    #[test]
    fn no_prefix_emits_all_unset_options() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![
            ("A", false),
            ("B", false),
            ("C", false),
        ];
        let _ = _options_unset(&mut state, &opts);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn each_unset_carries_unset_disp_label() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![("A", false), ("B", false)];
        let _ = _options_unset(&mut state, &opts);
        for m in &state.groups[0].matches {
            assert!(
                m.disp.as_deref().unwrap_or("").ends_with("(unset)"),
                "every unset option needs `(unset)` disp; got {:?}",
                m.disp
            );
        }
    }
}
