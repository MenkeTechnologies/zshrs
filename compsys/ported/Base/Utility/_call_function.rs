//! Port of `_call_function` — call a completion function by name.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_call_function`
//! (system copy `/opt/homebrew/share/zsh/functions/_call_function`).
//!
//! Upstream shell source (key lines from the 32-line fn):
//! ```text
//!  4  # Usage: _call_function <return> <name> [ <args> ... ]
//! 15  local _name _ret
//! 17  [[ "$1" != (|-) ]] && _name="$1"
//! 18  shift
//! 22  if (( ${+functions[$2]} )); then
//! 24    $2 "$@"
//! ```
//!
//! Upstream resolves a shell function by name (from `$functions`
//! associative array) and invokes it.
//!
//! Rust port: returns false. Resolving a Rust fn by string name
//! would require either a runtime fn-pointer registry (which we
//! don't have at the leaf) or shell integration. Caller-side
//! wiring would map names to Rust fn pointers and invoke directly.

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
