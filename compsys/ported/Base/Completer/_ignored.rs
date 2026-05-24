//! Port of `_ignored` — complete previously ignored matches. Moved
//! from `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _ignored - Complete previously ignored matches
pub fn _ignored(state: &mut CompletionState, ignored_patterns: &[String]) -> bool {
    // Would complete things that were ignored by fignore
    let _ = ignored_patterns;
    state.ignored > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_true_when_state_has_ignored_matches() {
        let mut state = CompletionState::new();
        state.ignored = 3;
        assert!(_ignored(&mut state, &[]));
    }

    #[test]
    fn returns_false_when_no_ignored_matches() {
        let mut state = CompletionState::new();
        assert!(!_ignored(&mut state, &[]));
    }
}
