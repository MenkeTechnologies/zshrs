//! Port of `_history` — complete from command history. Moved from
//! `compsys/functions.rs`.

use std::collections::HashSet;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _history - Complete from command history
pub fn _history(state: &mut CompletionState, history_entries: &[String]) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("history", true);
    let mut matched = false;
    let mut seen = HashSet::new();

    // Iterate in reverse (most recent first)
    for entry in history_entries.iter().rev() {
        if entry.starts_with(&prefix) && !seen.contains(entry) {
            state.add_match(Completion::new(entry), Some("history"));
            seen.insert(entry.clone());
            matched = true;
        }
    }

    state.end_group();
    matched
}
