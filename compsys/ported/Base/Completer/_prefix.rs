//! Port of `_prefix` — complete with prefix handling. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _prefix - Complete with prefix handling
pub fn _prefix(
    state: &mut CompletionState,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    // Save suffix, complete prefix only, restore
    let saved_suffix = state.params.suffix.clone();
    state.params.suffix.clear();

    let result = action(state);

    state.params.suffix = saved_suffix;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_cleared_during_action_and_restored_after() {
        let mut state = CompletionState::new();
        state.params.suffix = "BACK".into();
        let observed = std::cell::Cell::new(String::new());
        let result = _prefix(&mut state, |s| {
            // Snapshot what action sees.
            observed.set(s.params.suffix.clone());
            true
        });
        assert!(result);
        assert_eq!(
            observed.into_inner(),
            "",
            "action must see EMPTY suffix (prefix-only completion)"
        );
        assert_eq!(
            state.params.suffix, "BACK",
            "suffix must be restored after action returns"
        );
    }

    #[test]
    fn propagates_action_return_value() {
        let mut state = CompletionState::new();
        assert!(!_prefix(&mut state, |_| false), "false action -> false return");
        assert!(_prefix(&mut state, |_| true), "true action -> true return");
    }
}
