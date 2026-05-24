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
