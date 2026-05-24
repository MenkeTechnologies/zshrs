//! Port of `_match` — pattern-based matching. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::glob_match;

/// _match - Pattern-based matching
pub fn _match(state: &mut CompletionState, pattern: &str, candidates: &[String]) -> bool {
    let mut matched = false;

    for candidate in candidates {
        if glob_match(pattern, candidate) {
            state.add_match(Completion::new(candidate), None);
            matched = true;
        }
    }

    matched
}
