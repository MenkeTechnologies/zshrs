//! Port of `_nothing` — add no completions (but don't fail). Moved
//! from `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _nothing - Add no completions (but don't fail)
pub fn _nothing(_state: &mut CompletionState) -> bool {
    // Intentionally does nothing but returns success
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_true_without_emitting_matches() {
        let mut state = CompletionState::new();
        assert!(_nothing(&mut state));
        assert_eq!(state.nmatches, 0, "_nothing must NOT add matches");
        assert!(state.groups.is_empty(), "_nothing must NOT create groups");
    }
}
