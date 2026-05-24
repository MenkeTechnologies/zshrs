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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_iter_finds_first_match_after_prefix() {
        let mut state = CompletionState::new();
        state.params.prefix = "che".into();
        let history = vec!["ls -la".into(), "git checkout main".into()];
        assert!(_history_complete_word(&mut state, &history, 1));
    }

    #[test]
    fn backward_iter_walks_recent_first() {
        let mut state = CompletionState::new();
        state.params.prefix = "che".into();
        let history = vec![
            "git checkout old".into(),
            "git checkout new".into(),
        ];
        assert!(_history_complete_word(&mut state, &history, -1));
    }

    #[test]
    fn word_equal_to_prefix_skipped() {
        let mut state = CompletionState::new();
        state.params.prefix = "exactly".into();
        let history = vec!["exactly".into()];
        // Only word matching prefix IS the prefix itself → skipped
        // by `word != prefix` check.
        assert!(!_history_complete_word(&mut state, &history, 1));
    }
}
