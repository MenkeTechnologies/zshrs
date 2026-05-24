//! Port of `_parameters` — complete parameter (variable) names. Moved
//! from `compsys/library.rs`. Renamed from `parameters` to mirror zsh
//! shell function name `_parameters`.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _parameters - Complete parameter (variable) names
pub fn _parameters(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("parameters", true);

    for name in params.keys() {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("parameters"));
        }
    }

    state.end_group();
    state.nmatches > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_only_prefix_matching_keys() {
        let mut state = CompletionState::new();
        state.params.prefix = "HOM".into();
        let mut params = HashMap::new();
        params.insert("HOME".into(), "/root".into());
        params.insert("HOST".into(), "x".into());
        params.insert("USER".into(), "wizard".into());
        let ok = _parameters(&mut state, &params);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["HOME"], "HOST also starts with HO but not HOM; USER not in prefix");
    }

    #[test]
    fn empty_prefix_emits_all_keys() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("A".into(), "1".into());
        params.insert("B".into(), "2".into());
        let ok = _parameters(&mut state, &params);
        assert!(ok);
        assert_eq!(state.groups[0].matches.len(), 2);
    }

    #[test]
    fn returns_false_when_no_matches() {
        let mut state = CompletionState::new();
        state.params.prefix = "ZZZ".into();
        let mut params = HashMap::new();
        params.insert("HOME".into(), "/root".into());
        assert!(!_parameters(&mut state, &params));
    }
}
