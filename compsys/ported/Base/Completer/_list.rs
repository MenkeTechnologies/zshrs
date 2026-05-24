//! Port of `_list` — list completions without inserting. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _list - List completions without inserting
pub fn _list(state: &mut CompletionState) -> bool {
    state.params.compstate.list.push_str(" list");
    state.params.compstate.insert.clear();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_list_to_compstate_list_and_clears_insert() {
        let mut state = CompletionState::new();
        state.params.compstate.insert = "menu".into();
        assert!(_list(&mut state));
        assert!(state.params.compstate.list.contains("list"),
                "compstate[list] must gain `list` suffix");
        assert_eq!(state.params.compstate.insert, "",
                   "_list must clear compstate[insert] so completion only lists");
    }
}
