//! Port of `_options_unset` — complete currently unset options. Moved
//! from `compsys/library.rs`. Renamed from `options_unset` to mirror
//! zsh shell function name `_options_unset`.

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
