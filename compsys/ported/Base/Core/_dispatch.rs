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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_command_returns_matched() {
        let mut state = MainCompleteState::new("git status", 10);
        let mut comps = HashMap::new();
        comps.insert("git".to_string(), "_git".to_string());
        let r = _dispatch(&mut state, &comps, &["git"]);
        assert!(matches!(r, CompleterResult::Matched));
    }

    #[test]
    fn unknown_commands_return_nomatch() {
        let mut state = MainCompleteState::new("x", 1);
        let comps = HashMap::new();
        let r = _dispatch(&mut state, &comps, &["x", "y"]);
        assert!(matches!(r, CompleterResult::NoMatch));
    }

    #[test]
    fn first_matching_command_short_circuits() {
        let mut state = MainCompleteState::new("a", 1);
        let mut comps = HashMap::new();
        comps.insert("b".to_string(), "_b".to_string());
        // a is missing → skipped; b matches → Matched returned.
        let r = _dispatch(&mut state, &comps, &["a", "b", "c"]);
        assert!(matches!(r, CompleterResult::Matched));
    }
}
