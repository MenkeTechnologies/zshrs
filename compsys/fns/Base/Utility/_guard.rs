//! Port of `_guard` — guard against completing in wrong context. Moved
//! from `compsys/functions.rs`.

use crate::base::MainCompleteState;

use super::shared::glob_match;

/// _guard - Guard against completing in wrong context
pub fn _guard(state: &MainCompleteState, pattern: &str) -> bool {
    let prefix = state.comp.params.prefix.clone();

    // Simple glob matching
    if pattern.contains('*') || pattern.contains('?') {
        glob_match(pattern, &prefix)
    } else {
        prefix.starts_with(pattern)
    }
}
