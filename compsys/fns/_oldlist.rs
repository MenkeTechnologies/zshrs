//! Port of `_oldlist` — use previous completion list. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _oldlist - Use previous completion list
pub fn _oldlist(state: &mut CompletionState) -> bool {
    state.params.compstate.old_list = "keep".to_string();
    true
}
