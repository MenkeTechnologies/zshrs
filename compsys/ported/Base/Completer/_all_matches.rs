//! Port of `_all_matches` — show all possible matches. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _all_matches - Show all possible matches
pub fn _all_matches(state: &mut CompletionState) -> bool {
    // Just show all matches without filtering
    state.params.compstate.insert = "all".to_string();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_compstate_insert_to_all() {
        let mut state = CompletionState::new();
        assert!(_all_matches(&mut state));
        assert_eq!(state.params.compstate.insert, "all");
    }

    #[test]
    fn overwrites_existing_insert_value() {
        let mut state = CompletionState::new();
        state.params.compstate.insert = "menu".to_string();
        assert!(_all_matches(&mut state));
        assert_eq!(state.params.compstate.insert, "all");
    }
}
