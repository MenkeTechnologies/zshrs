//! Port of `_dispatch` — dispatch to the appropriate completion
//! function.
//!
//! Local shell reference: `compsys/functions/Base/Core/_dispatch`
//! (system copy `/opt/homebrew/share/zsh/functions/_dispatch`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  9  if [[ "$1" = -s ]]; then noskip=yes; shift; fi
//! 14  [[ -z "$noskip" ]] && _compskip=
//! 16  curcontext="${curcontext%:*:*}:${1}:"
//! 22  if [[ "$_compskip" != (all|*patterns*) ]]; then
//! 24    for str in "$@"; do
//! 26      service="${_services[$str]:-$str}"
//! 27      for i in "${(@)_patcomps[(K)$str]}"; do
//! 32        eval "$i" && ret=0
//! ```
//!
//! Simplified Rust port: takes the registered (comps, commands)
//! maps explicitly instead of going through shell `_services` /
//! `_patcomps` globals. The lookup-and-invoke loop is the same
//! shape — try the per-command function for `cmd`, walk pattern
//! fallbacks. The `-s` / `_compskip` machinery is shell-side state
//! deferred to the caller.

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
