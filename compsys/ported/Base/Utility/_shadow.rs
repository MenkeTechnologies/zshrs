//! Port of `_shadow` — shadow existing completions.
//!
//! Local shell reference: NO standalone shell file `_shadow`
//! exists upstream. The shell-side `_shadow` is a per-call utility
//! invoked by `_complete_help` to TEMPORARILY redefine
//! `compadd`/`compcall`/`zstyle` so it can RECORD what zstyle
//! lookups + compadd invocations a completer would make, without
//! actually emitting matches.
//!
//! Simplified Rust port: takes an action closure and runs it
//! verbatim. The "shadowing" behavior (intercepting compadd to
//! record instead of emit) would require trait-object-based
//! receiver swap-out — left to the caller that wires up the
//! recorder.

use crate::compcore::CompletionState;

/// _shadow - Shadow existing completions
pub fn _shadow(
    state: &mut CompletionState,
    _shadow_name: &str,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    // Shadow mechanism - run action in isolated context
    action(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_action() {
        let mut state = CompletionState::new();
        assert!(_shadow(&mut state, "x", |_| true));
        assert!(!_shadow(&mut state, "x", |_| false));
    }
}
