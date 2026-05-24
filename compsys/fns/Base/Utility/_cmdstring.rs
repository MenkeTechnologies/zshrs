//! Port of `_cmdstring` — complete a command string (for eval, etc.).
//! Moved from `compsys/library.rs`.

use crate::compcore::CompletionState;

use super::_command_names::{_command_names, ShellInventory};

/// _cmdstring - Complete a command string (for eval, etc.)
pub fn _cmdstring(state: &mut CompletionState, inv: &ShellInventory<'_>) -> bool {
    // Complete as if it were a command line
    _command_names(state, inv, false)
}
