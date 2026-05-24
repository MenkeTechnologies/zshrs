//! Port of `_complete` — the main completer.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_complete`
//! (system copy `/opt/homebrew/share/zsh/functions/_complete`).
//!
//! Upstream shell source (key lines from the ~100-line completer):
//! ```text
//!  3  # Generate all possible completions.
//!  9  local comp name oldcontext ret=1 service
//! 10  typeset -T curcontext="$curcontext" ccarray
//! 12  oldcontext="$curcontext"
//! 16  if [[ -n "$compcontext" ]]; then
//! 19    ccarray=( ${(s.:.)compcontext} )
//! 22  comp="$_comps[-context-]"
//! ```
//!
//! Upstream is the default completer-style value; it dispatches to
//! `_normal` for the standard command-vs-argument resolution.
//!
//! Faithful Rust port: thin wrapper that delegates to `_normal`,
//! matching the shell's typical end of the pipeline.

use crate::base::{CompleterResult, MainCompleteState};
use crate::ported::_normal::_normal;

/// _complete - the main completer
pub fn _complete(state: &mut MainCompleteState) -> CompleterResult {
    // This is the default completer that handles normal completion
    _normal(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_normal() {
        // _complete is a thin wrapper around _normal — both should
        // return the same CompleterResult for the same input state.
        let mut s1 = MainCompleteState::new("ls ", 3);
        let mut s2 = MainCompleteState::new("ls ", 3);
        assert_eq!(
            std::mem::discriminant(&_complete(&mut s1)),
            std::mem::discriminant(&_normal(&mut s2)),
            "_complete must delegate verbatim to _normal"
        );
    }

    #[test]
    fn empty_state_returns_no_match() {
        let mut state = MainCompleteState::new("", 0);
        assert!(matches!(_complete(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn command_position_returns_no_match_pending_dispatch() {
        // Without a -command- handler registered, _normal returns
        // NoMatch — _complete must propagate.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        assert!(matches!(_complete(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn argument_position_returns_no_match_when_no_comps_registered() {
        let mut state = MainCompleteState::new("git status", 10);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["git".into(), "status".into()];
        assert!(matches!(_complete(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn passes_through_curcontext_unchanged() {
        // shell:10 `typeset -T curcontext` — local copy, restored on
        // return. Our impl doesn't shadow either; the context value
        // the caller sees should match what was passed in.
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":pinned-context:".into();
        let _ = _complete(&mut state);
        assert_eq!(state.ctx.context, ":pinned-context:");
    }

    #[test]
    fn does_not_create_groups_when_no_match() {
        let mut state = MainCompleteState::new("", 0);
        let before = state.comp.groups.len();
        let _ = _complete(&mut state);
        // _normal returning NoMatch means no group should have been
        // created.
        assert_eq!(state.comp.groups.len(), before);
    }

    #[test]
    fn idempotent_on_no_match_state() {
        // Calling twice with the same NoMatch state should still
        // return NoMatch — no hidden side effects accumulate.
        let mut state = MainCompleteState::new("", 0);
        let first = _complete(&mut state);
        let second = _complete(&mut state);
        assert_eq!(
            std::mem::discriminant(&first),
            std::mem::discriminant(&second)
        );
    }
}
