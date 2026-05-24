//! Port of `_approximate` — approximate/fuzzy matching. Moved from
//! `compsys/functions.rs`.

use crate::base::{CompleterResult, MainCompleteState};
use crate::completion::Completion;

use super::shared::edit_distance;

/// _approximate - Approximate/fuzzy matching
pub fn _approximate(state: &mut MainCompleteState, max_errors: usize) -> CompleterResult {
    let original = state.comp.params.prefix.clone();

    // Get all potential matches and filter by edit distance
    // This is a simplified implementation
    let matches: Vec<String> = state
        .comp
        .all_completions()
        .iter()
        .filter(|c| edit_distance(&original, &c.str_) <= max_errors)
        .map(|c| c.str_.clone())
        .collect();

    if matches.is_empty() {
        CompleterResult::NoMatch
    } else {
        for m in matches {
            state.comp.add_match(Completion::new(&m), None);
        }
        CompleterResult::Matched
    }
}
