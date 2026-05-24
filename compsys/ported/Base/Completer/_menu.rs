//! Port of `_menu` — menu completion mode. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _menu - Menu completion mode
pub fn _menu(state: &mut CompletionState) -> bool {
    state.params.compstate.insert = "menu".to_string();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_compstate_insert_to_menu() {
        let mut state = CompletionState::new();
        assert!(_menu(&mut state));
        assert_eq!(state.params.compstate.insert, "menu");
    }
}
