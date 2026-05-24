//! Port of `_list` — list completions without inserting. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _list - List completions without inserting
pub fn _list(state: &mut CompletionState) -> bool {
    state.params.compstate.list.push_str(" list");
    state.params.compstate.insert.clear();
    true
}
