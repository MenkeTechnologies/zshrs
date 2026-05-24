//! Port of `_precommand` — complete after a precommand (sudo, nohup,
//! etc.). Moved from `compsys/library.rs`. Renamed from `precommand` to
//! mirror zsh shell function name `_precommand`.

use crate::base::{_normal, CompleterResult, MainCompleteState};

/// _precommand - Complete after a precommand (sudo, nohup, etc.)
pub fn _precommand(state: &mut MainCompleteState) -> bool {
    // Skip the precommand and complete as normal command
    if state.comp.params.current > 1 {
        // Treat rest as command line
        matches!(_normal(state), CompleterResult::Matched)
    } else {
        false
    }
}
