//! Port of `_user_expand` — user-defined expansions. Moved from
//! `compsys/functions.rs`.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _user_expand - User-defined expansions
pub fn _user_expand(state: &mut CompletionState, expansions: &HashMap<String, String>) -> bool {
    let prefix = state.params.prefix.clone();

    let mut matched = false;
    for (pattern, expansion) in expansions {
        if prefix.starts_with(pattern) {
            let expanded = prefix.replacen(pattern, expansion, 1);
            state.add_match(Completion::new(&expanded), None);
            matched = true;
        }
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_at_prefix_start_replaces_once() {
        let mut state = CompletionState::new();
        state.params.prefix = "WORK/proj".into();
        let mut exp = HashMap::new();
        exp.insert("WORK".into(), "/home/wizard/work".into());
        let ok = _user_expand(&mut state, &exp);
        assert!(ok);
        let m = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .next()
            .expect("a match");
        assert_eq!(m.str_, "/home/wizard/work/proj");
    }

    #[test]
    fn pattern_only_matches_at_start_not_middle() {
        let mut state = CompletionState::new();
        state.params.prefix = "/proj/WORK".into();
        let mut exp = HashMap::new();
        exp.insert("WORK".into(), "/home/work".into());
        // `WORK` IS in the prefix but not at the start → no expand.
        assert!(!_user_expand(&mut state, &exp));
    }

    #[test]
    fn replacen_means_pattern_replaced_only_once_in_expansion() {
        // If pattern appears in expansion text, prove we don't loop.
        let mut state = CompletionState::new();
        state.params.prefix = "X".into();
        let mut exp = HashMap::new();
        exp.insert("X".into(), "XX".into());
        let ok = _user_expand(&mut state, &exp);
        assert!(ok);
        let m = &state.groups[0].matches[0];
        assert_eq!(m.str_, "XX", "first-only replacement, not recursive");
    }
}
