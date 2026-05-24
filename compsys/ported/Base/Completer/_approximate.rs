//! Port of `_approximate` — approximate/fuzzy matching. Moved from
//! `compsys/functions.rs`.

use crate::base::{CompleterResult, MainCompleteState};
use crate::completion::Completion;

use super::shared::edit_distance;

/// _approximate - Approximate/fuzzy matching
pub fn _approximate(state: &mut MainCompleteState, max_errors: usize) -> CompleterResult {
    let original = state.comp.params.prefix.clone();

    // Get all potential matches and filter by edit distance
    // This is a simplified implementation
    let matches: Vec<String> = state
        .comp
        .all_completions()
        .iter()
        .filter(|c| edit_distance(&original, &c.str_) <= max_errors)
        .map(|c| c.str_.clone())
        .collect();

    if matches.is_empty() {
        CompleterResult::NoMatch
    } else {
        for m in matches {
            state.comp.add_match(Completion::new(&m), None);
        }
        CompleterResult::Matched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_typo_within_max_errors_matches() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "comit".into();
        state.comp.add_match(Completion::new("commit"), None);
        state.comp.add_match(Completion::new("checkout"), None);
        match _approximate(&mut state, 1) {
            CompleterResult::Matched => {}
            other => panic!("expected Matched, got {other:?}"),
        }
    }

    #[test]
    fn beyond_max_errors_returns_no_match() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "totally-different".into();
        state.comp.add_match(Completion::new("xyz"), None);
        assert!(matches!(
            _approximate(&mut state, 1),
            CompleterResult::NoMatch
        ));
    }

    #[test]
    fn zero_max_errors_only_exact() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "git".into();
        state.comp.add_match(Completion::new("git"), None);
        state.comp.add_match(Completion::new("gut"), None);
        // max_errors=0 → only the exact "git" passes the filter.
        assert!(matches!(
            _approximate(&mut state, 0),
            CompleterResult::Matched
        ));
    }
}
