//! Port of `_nothing` — add no completions (but don't fail).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_nothing`
//! (system copy `/opt/homebrew/share/zsh/functions/_nothing`).
//!
//! Upstream shell source (the WHOLE file, 3 lines):
//! ```text
//!  1  #compdef true false log times clear logname whoami sync
//!  3  _message 'no argument or option'
//! ```
//!
//! Upstream is bound as the completion for commands that genuinely
//! take no args (`true`, `false`, `whoami`, etc.) — it emits a
//! single "no argument or option" message via `_message`.
//!
//! Simplified Rust port: returns true without adding matches or
//! a message. (The message would need a CompletionState writer to
//! emit — the Rust impl signals "completion happened, just no
//! results" by returning true.)

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
