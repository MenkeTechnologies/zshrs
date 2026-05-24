//! Port of `_as_if` — complete as if in different context.
//!
//! Local shell reference: NO standalone `_as_if` shell file exists
//! upstream. The name `_as_if_command` IS a shell helper used by
//! the `_precommand` family — they temporarily push a different
//! `curcontext` then run the wrapped completer.
//!
//! Rust port: takes the override context + action closure and
//! swaps `state.ctx.context` for the duration of the call,
//! restoring afterward. Same shape as the shell helper, just
//! exposed as a generic closure-runner rather than a
//! one-shot-per-precommand helper.

use crate::base::MainCompleteState;

/// _as_if - Complete as if in different context
pub fn _as_if(
    state: &mut MainCompleteState,
    new_context: &str,
    action: impl FnOnce(&mut MainCompleteState) -> bool,
) -> bool {
    let old_context = state.ctx.context.clone();
    state.ctx.context = new_context.to_string();

    let result = action(state);

    state.ctx.context = old_context;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_swapped_during_action_restored_after() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::orig:".into();
        let observed = std::cell::Cell::new(String::new());
        _as_if(&mut state, ":complete::shadow:", |s| {
            observed.set(s.ctx.context.clone());
            true
        });
        assert_eq!(observed.into_inner(), ":complete::shadow:");
        assert_eq!(
            state.ctx.context, ":complete::orig:",
            "context must be restored after action"
        );
    }

    #[test]
    fn propagates_action_return_value() {
        let mut state = MainCompleteState::new("", 0);
        assert!(_as_if(&mut state, "x", |_| true));
        assert!(!_as_if(&mut state, "x", |_| false));
    }
}
