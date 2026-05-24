//! Port of `_ignored` — complete previously ignored matches.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_ignored`
//! (system copy `/opt/homebrew/share/zsh/functions/_ignored`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  5  [[ _matcher_num -gt 1 || $compstate[ignored] -eq 0 ]] && return 1
//!  7  local comp
//!  9  if ! zstyle -a ":completion:${curcontext}:" completer comp; then
//! 10    comp=( "${(@)_completers[1,_completer_num-1]}" )
//! 11    ind=${comp[(I)_ignored(|:*)]}
//! 14  local _comp_no_ignore=yes …
//! ```
//!
//! The shell version re-runs the preceding completers with
//! `_comp_no_ignore=yes` set so they emit the matches that had been
//! filtered out by `ignored-patterns` zstyle.
//!
//! Simplified Rust port: gates on `state.ignored > 0` (the count of
//! previously-ignored matches) and returns true to signal "we should
//! show these ignored matches now". Re-running completers under
//! `_comp_no_ignore=yes` requires the full completer dispatch loop;
//! caller wires that.

use crate::compcore::CompletionState;

/// _ignored - Complete previously ignored matches
pub fn _ignored(state: &mut CompletionState, ignored_patterns: &[String]) -> bool {
    // Would complete things that were ignored by fignore
    let _ = ignored_patterns;
    state.ignored > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_true_when_state_has_ignored_matches() {
        let mut state = CompletionState::new();
        state.ignored = 3;
        assert!(_ignored(&mut state, &[]));
    }

    #[test]
    fn returns_false_when_no_ignored_matches() {
        let mut state = CompletionState::new();
        assert!(!_ignored(&mut state, &[]));
    }

    #[test]
    fn ignored_count_zero_means_false() {
        let mut state = CompletionState::new();
        state.ignored = 0;
        assert!(!_ignored(&mut state, &["pat".into()]));
    }

    #[test]
    fn patterns_arg_currently_unused_does_not_panic() {
        let mut state = CompletionState::new();
        state.ignored = 5;
        // patterns arg is documented as "not yet wired" — pin it
        // doesn't crash with various shapes.
        assert!(_ignored(&mut state, &[]));
        assert!(_ignored(&mut state, &["a".into()]));
        assert!(_ignored(&mut state, &["a".into(), "b".into(), "c".into()]));
    }

    #[test]
    fn high_ignored_count_still_returns_true() {
        let mut state = CompletionState::new();
        state.ignored = 999_999;
        assert!(_ignored(&mut state, &[]));
    }
}
