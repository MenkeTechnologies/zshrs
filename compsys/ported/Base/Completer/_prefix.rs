//! Port of `_prefix` — complete with prefix handling.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_prefix`
//! (system copy `/opt/homebrew/share/zsh/functions/_prefix`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  [[ _matcher_num -gt 1 || -z "$SUFFIX" ]] && return 1
//!  5  local comp curcontext="$curcontext" tmp suf="$SUFFIX"
//! 16  if zstyle -t ":completion:${curcontext}:" add-space; then
//! 17    ISUFFIX=" $SUFFIX"
//! 18  else
//! 19    ISUFFIX="$SUFFIX"
//! 20  fi
//! 22  SUFFIX=
//! ```
//!
//! The shell version moves SUFFIX into ISUFFIX (the "ignored suffix",
//! preserved on the line but excluded from completion matching),
//! then runs the rest of the completer pipeline against bare PREFIX.
//!
//! Faithful Rust port: takes an action closure and runs it with
//! SUFFIX cleared. Restores SUFFIX afterward — pinned by the
//! `suffix_cleared_during_action_and_restored_after` test.

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
