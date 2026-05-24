//! Port of `_ignored` — complete previously ignored matches. Moved
//! from `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _ignored - Complete previously ignored matches
pub fn _ignored(state: &mut CompletionState, ignored_patterns: &[String]) -> bool {
    // Would complete things that were ignored by fignore
    let _ = ignored_patterns;
    state.ignored > 0
}
