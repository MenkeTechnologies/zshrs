//! Port of `_expand_word` — expand word (aliases, variables, etc.).
//! Moved from `compsys/functions.rs`.

use crate::compcore::CompletionState;

use super::_expand::_expand;

/// _expand_word - Expand word (aliases, variables, etc.)
pub fn _expand_word(state: &mut CompletionState) -> bool {
    _expand(state)
}
