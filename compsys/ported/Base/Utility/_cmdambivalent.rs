//! Port of `_cmdambivalent` — handle commands that can be run with or
//! without arguments. Moved from `compsys/library.rs`.

use crate::base::MainCompleteState;

use super::_command_names::{_command_names, ShellInventory};

/// _cmdambivalent - Handle commands that can be run with or without arguments
pub fn _cmdambivalent(state: &mut MainCompleteState, inv: &ShellInventory<'_>) -> bool {
    // If no arguments yet, complete as command
    if state.comp.params.current <= 1 {
        _command_names(&mut state.comp, inv, false)
    } else {
        // Otherwise use normal completion
        true
    }
}
