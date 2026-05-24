//! Port of `_all_matches` — show all possible matches. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _all_matches - Show all possible matches
pub fn _all_matches(state: &mut CompletionState) -> bool {
    // Just show all matches without filtering
    state.params.compstate.insert = "all".to_string();
    true
}
