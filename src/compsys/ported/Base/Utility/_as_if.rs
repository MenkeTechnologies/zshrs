//! Port of `_as_if` from `Completion/Base/Utility/_as_if`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  local words=("$words[@]") CURRENT=$CURRENT
//! sh: 3  local _comp_command1 _comp_command2 _comp_command
//! sh: 4
//! sh: 5  words[1]=("$@")
//! sh: 6  (( CURRENT += $# - 1 ))
//! sh: 7
//! sh: 8  _set_command
//! sh: 9
//! sh:10  _dispatch "$_comp_command" "$_comp_command1" "$_comp_command2" -default-
//! ```



use crate::compsys::base::MainCompleteState;

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

    #[test]
    fn restore_runs_even_on_action_returning_false() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":orig:".into();
        _as_if(&mut state, ":new:", |_| false);
        assert_eq!(state.ctx.context, ":orig:", "context restored even on false");
    }

    #[test]
    fn empty_context_override_replaces_to_empty_then_restores() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":orig:".into();
        let saw = std::cell::Cell::new(String::new());
        _as_if(&mut state, "", |s| {
            saw.set(s.ctx.context.clone());
            true
        });
        assert_eq!(saw.into_inner(), "");
        assert_eq!(state.ctx.context, ":orig:");
    }

    #[test]
    fn nested_as_if_restore_in_lifo_order() {
        // Inner _as_if changes context; on its return outer's
        // context is still the OUTER override, NOT the original.
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":root:".into();
        _as_if(&mut state, ":outer:", |s| {
            assert_eq!(s.ctx.context, ":outer:");
            _as_if(s, ":inner:", |t| {
                assert_eq!(t.ctx.context, ":inner:");
                true
            });
            // After inner returns, we're back to outer's context.
            assert_eq!(s.ctx.context, ":outer:");
            true
        });
        assert_eq!(state.ctx.context, ":root:");
    }

    #[test]
    fn action_can_mutate_state_other_than_context() {
        // Pin that mutations to non-context fields survive.
        let mut state = MainCompleteState::new("", 0);
        _as_if(&mut state, ":shadow:", |s| {
            s.comp.params.prefix = "set-by-as-if".into();
            true
        });
        assert_eq!(state.comp.params.prefix, "set-by-as-if");
    }

    #[test]
    fn context_with_special_chars_survives_roundtrip() {
        let original = ":complete::cmd[git]::tag(*):value:";
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = original.into();
        _as_if(&mut state, ":new:", |_| true);
        assert_eq!(state.ctx.context, original);
    }
}
