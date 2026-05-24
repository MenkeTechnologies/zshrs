//! Port of `_dispatch` — dispatch to the appropriate completion
//! function. Moved from `compsys/functions.rs`. Renamed from
//! `dispatch` to mirror zsh shell function name `_dispatch`.

use std::collections::HashMap;

use crate::base::{CompleterResult, MainCompleteState};

/// _dispatch - Dispatch to the appropriate completion function
pub fn _dispatch(
    _state: &mut MainCompleteState,
    comps: &HashMap<String, String>,
    commands: &[&str],
) -> CompleterResult {
    for cmd in commands {
        if let Some(func) = comps.get(*cmd) {
            // In real implementation, would call the function
            // For now, return that we found it
            let _ = func;
            return CompleterResult::Matched;
        }
    }
    CompleterResult::NoMatch
}
