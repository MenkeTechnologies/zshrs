//! Port of `_complete` — the main completer.
//!
//! Extracted from `compsys/base.rs` (was lines ~671-674). Mirrors zsh
//! upstream `Completion/Base/Completer/_complete`. The default
//! completer entry point invoked by `_main_complete`; delegates to
//! `_normal` for command-vs-argument dispatch.

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
}
