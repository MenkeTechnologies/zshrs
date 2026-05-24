//! Port of `_nothing` — add no completions (but don't fail). Moved
//! from `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _nothing - Add no completions (but don't fail)
pub fn _nothing(_state: &mut CompletionState) -> bool {
    // Intentionally does nothing but returns success
    true
}
