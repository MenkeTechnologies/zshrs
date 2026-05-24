//! Port of `_oldlist` — use previous completion list. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _oldlist - Use previous completion list
pub fn _oldlist(state: &mut CompletionState) -> bool {
    state.params.compstate.old_list = "keep".to_string();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_old_list_to_keep() {
        let mut state = CompletionState::new();
        assert!(_oldlist(&mut state));
        assert_eq!(state.params.compstate.old_list, "keep",
                   "shell `compstate[old_list]=keep` must be reflected");
    }
}
