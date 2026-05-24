//! Port of `_user_expand` — user-defined expansions. Moved from
//! `compsys/functions.rs`.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _user_expand - User-defined expansions
pub fn _user_expand(state: &mut CompletionState, expansions: &HashMap<String, String>) -> bool {
    let prefix = state.params.prefix.clone();

    let mut matched = false;
    for (pattern, expansion) in expansions {
        if prefix.starts_with(pattern) {
            let expanded = prefix.replacen(pattern, expansion, 1);
            state.add_match(Completion::new(&expanded), None);
            matched = true;
        }
    }

    matched
}
