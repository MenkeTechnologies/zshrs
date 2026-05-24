//! Port of `_expand_alias` — expand aliases. Moved from
//! `compsys/functions.rs`.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};

/// _expand_alias - Expand aliases
pub fn _expand_alias(state: &mut CompletionState, aliases: &HashMap<String, String>) -> bool {
    let word = state.params.current_word();

    if let Some(expansion) = aliases.get(&word) {
        let mut comp = Completion::new(expansion);
        comp.flags |= CompletionFlags::NOSPACE;
        state.add_match(comp, None);
        true
    } else {
        false
    }
}
