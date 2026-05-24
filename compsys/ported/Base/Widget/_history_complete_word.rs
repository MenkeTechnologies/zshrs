//! Port of `_history_complete_word` — complete word from history.
//! Moved from `compsys/functions.rs`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _history_complete_word - Complete word from history
pub fn _history_complete_word(
    state: &mut CompletionState,
    history_entries: &[String],
    direction: i32, // -1 = backward, 1 = forward
) -> bool {
    let prefix = state.params.prefix.clone();

    let iter: Box<dyn Iterator<Item = &String>> = if direction < 0 {
        Box::new(history_entries.iter().rev())
    } else {
        Box::new(history_entries.iter())
    };

    for entry in iter {
        // Find words in entry that match prefix
        for word in entry.split_whitespace() {
            if word.starts_with(&prefix) && word != prefix {
                state.add_match(Completion::new(word), None);
                return true;
            }
        }
    }

    false
}
