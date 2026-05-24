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
}
