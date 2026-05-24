//! Port of `_correct_word` — correct word spelling. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::edit_distance;

/// _correct_word - Correct word spelling
pub fn _correct_word(state: &mut CompletionState, words: &[String]) -> bool {
    let prefix = state.params.prefix.clone();

    let mut matched = false;
    for word in words {
        if edit_distance(&prefix, word) <= 2 {
            state.add_match(Completion::new(word), None);
            matched = true;
        }
    }

    matched
}
