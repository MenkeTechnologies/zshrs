//! Port of `_options_set` from `Completion/Zsh/Type/_options_set`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete all set options. This relies on `_main_complete' to store the
//! sh: 4  # names of the options that were set when it was called in the array
//! sh: 5  # `_options_set'.
//! sh: 6
//! sh: 7  local expl
//! sh: 8
//! sh: 9  _wanted zsh-options expl 'set zsh option' \
//! sh:10      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -a - _options_set
//! ```
//!
//! Upstream uses `${(@k)options[(R)on]}` to get the keys of the
//! shell `$options` array whose VALUE is "on" (i.e., set).
//!
//! Faithful Rust port: filters the caller's `&[(name, is_set)]`
//! slice to set-only entries and delegates to `_options` for the
//! actual emit (so disp formatting stays consistent).



use crate::compsys::compcore::CompletionState;

use super::_options::_options;

/// _options_set - Complete currently set options
pub fn _options_set(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let set_opts: Vec<(&str, bool)> = shell_options
        .iter()
        .filter(|(_, is_set)| *is_set)
        .copied()
        .collect();
    _options(state, &set_opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_to_set_only() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![
            ("EXTENDED_GLOB", true),
            ("NULL_GLOB", false),
            ("PIPE_FAIL", true),
            ("NO_BANG_HIST", false),
        ];
        let ok = _options_set(&mut state, &opts);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"EXTENDED_GLOB"));
        assert!(names.contains(&"PIPE_FAIL"));
        assert!(!names.contains(&"NULL_GLOB"), "unset option leaked through filter");
        assert!(!names.contains(&"NO_BANG_HIST"));
    }

    #[test]
    fn returns_false_when_no_set_options() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![("A", false), ("B", false)];
        assert!(!_options_set(&mut state, &opts));
    }

    #[test]
    fn empty_options_list_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_options_set(&mut state, &[]));
    }

    #[test]
    fn prefix_filters_combined_with_set_filter() {
        let mut state = CompletionState::new();
        state.params.prefix = "EXT".into();
        let opts: Vec<(&str, bool)> = vec![
            ("EXTENDED_GLOB", true),
            ("EXTENDED_HISTORY", false), // unset → excluded
            ("EXTRA_VERBOSE", true),
            ("PIPE_FAIL", true), // off-prefix → excluded
        ];
        let ok = _options_set(&mut state, &opts);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"EXTENDED_GLOB"));
        assert!(names.contains(&"EXTRA_VERBOSE"));
        assert!(!names.contains(&"EXTENDED_HISTORY"));
        assert!(!names.contains(&"PIPE_FAIL"));
    }

    #[test]
    fn no_typo_in_disp_for_set_options() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![("MAGIC", true)];
        _options_set(&mut state, &opts);
        let m = &state.groups[0].matches[0];
        assert_eq!(m.disp.as_deref(), Some("MAGIC (set)"));
    }

    #[test]
    fn empty_prefix_emits_all_set_options() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![
            ("A", true),
            ("B", true),
            ("C", true),
        ];
        let _ = _options_set(&mut state, &opts);
        assert_eq!(state.groups[0].matches.len(), 3);
    }

    #[test]
    fn complement_of_options_unset() {
        // Pin that _options_set + _options_unset partition the input.
        use crate::compsys::ported::_options_unset::_options_unset;
        let opts: Vec<(&str, bool)> = vec![
            ("A", true),
            ("B", false),
            ("C", true),
            ("D", false),
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
        assert!(set.is_disjoint(&unset));
        assert_eq!(set.len() + unset.len(), opts.len());
    }

    #[test]
    fn all_set_options_show_disp_set_label() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![("A", true), ("B", true)];
        let _ = _options_set(&mut state, &opts);
        for m in &state.groups[0].matches {
            assert!(
                m.disp.as_deref().unwrap_or("").ends_with("(set)"),
                "every set option should have `(set)` disp; got {:?}",
                m.disp
            );
        }
    }
}
