//! Port of `_options_set` — complete currently set options. Moved from
//! `compsys/library.rs`. Renamed from `options_set` to mirror zsh shell
//! function name `_options_set`.

use crate::compcore::CompletionState;

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
}
