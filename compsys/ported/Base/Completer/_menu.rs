//! Port of `_menu` — menu completion mode. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _menu - Menu completion mode
pub fn _menu(state: &mut CompletionState) -> bool {
    state.params.compstate.insert = "menu".to_string();
    true
}
