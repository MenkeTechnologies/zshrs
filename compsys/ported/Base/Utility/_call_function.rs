//! Port of `_call_function` — call a completion function by name.
//! Moved from `compsys/functions.rs`. Renamed from `call_function` to
//! mirror zsh shell function name `_call_function`.

use crate::base::MainCompleteState;

/// _call_function - Call a completion function by name
pub fn _call_function(_state: &mut MainCompleteState, _func: &str) -> bool {
    // Would look up and call the function
    // Needs shell integration
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_false_without_shell_integration() {
        // The shell-side _call_function resolves a function name
        // from `$functions` and runs it. The Rust leaf has no shell
        // table to resolve from — pin that we return false
        // unconditionally so a future "sometimes-true" change
        // surfaces deliberately.
        let mut state = MainCompleteState::new("", 0);
        assert!(!_call_function(&mut state, "any-name"));
    }
}
