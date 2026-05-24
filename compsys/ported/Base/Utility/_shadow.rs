//! Port of `_shadow` — shadow existing completions.
//!
//! Local shell reference: NO standalone shell file `_shadow`
//! exists upstream. The shell-side `_shadow` is a per-call utility
//! invoked by `_complete_help` to TEMPORARILY redefine
//! `compadd`/`compcall`/`zstyle` so it can RECORD what zstyle
//! lookups + compadd invocations a completer would make, without
//! actually emitting matches.
//!
//! Simplified Rust port: takes an action closure and runs it
//! verbatim. The "shadowing" behavior (intercepting compadd to
//! record instead of emit) would require trait-object-based
//! receiver swap-out — left to the caller that wires up the
//! recorder.

use crate::compcore::CompletionState;

/// _shadow - Shadow existing completions
pub fn _shadow(
    state: &mut CompletionState,
    _shadow_name: &str,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    // Shadow mechanism - run action in isolated context
    action(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_action() {
        let mut state = CompletionState::new();
        assert!(_shadow(&mut state, "x", |_| true));
        assert!(!_shadow(&mut state, "x", |_| false));
    }

    #[test]
    fn action_can_mutate_state() {
        use crate::completion::Completion;
        let mut state = CompletionState::new();
        _shadow(&mut state, "shadow1", |s| {
            s.add_match(Completion::new("via-shadow"), None);
            true
        });
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"via-shadow".to_string()));
    }

    #[test]
    fn shadow_name_is_passed_but_currently_ignored() {
        // Pin that the shadow_name arg is accepted (signature stability)
        // but doesn't gate execution.
        let mut state = CompletionState::new();
        assert!(_shadow(&mut state, "any-arbitrary-name", |_| true));
    }

    #[test]
    fn action_observes_provided_state() {
        let mut state = CompletionState::new();
        state.params.prefix = "x".into();
        let seen = std::cell::Cell::new(String::new());
        _shadow(&mut state, "s", |s| {
            seen.set(s.params.prefix.clone());
            true
        });
        assert_eq!(seen.into_inner(), "x");
    }
}
