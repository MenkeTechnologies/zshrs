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
