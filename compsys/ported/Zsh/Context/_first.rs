//! Port of `_first` — `-first-` context handler (runs before any
//! other context fn).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_first`.
//!
//! Upstream shell source (the WHOLE file is comments / example):
//! ```text
//! #compdef -first-
//!
//! # This function is called at the very beginning before any other
//! # function for a specific context.
//! #
//! # This just gives some examples of things you might want to do here.
//! ```
//!
//! Upstream `_first` is a USER-OVERRIDE HOOK — the default body is
//! empty (only comments). End users redefine it to customize the
//! first-thing-that-runs behavior.
//!
//! Strict Rust port: returns false (no first-thing override active).
//! Calling code can replace the default by calling a registered
//! `_first` fn via [`crate::ported::_call_function`].

use crate::base::MainCompleteState;

/// `_first` — default hook (no-op). Returns false so the dispatch
/// loop continues to the next context handler.
pub fn _first(_state: &mut MainCompleteState) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_first(&mut state));
    }

    #[test]
    fn does_not_mutate_state() {
        let mut state = MainCompleteState::new("hello", 5);
        state.comp.params.prefix = "p".into();
        state.comp.params.suffix = "s".into();
        let _ = _first(&mut state);
        assert_eq!(state.comp.params.prefix, "p");
        assert_eq!(state.comp.params.suffix, "s");
    }

    #[test]
    fn does_not_create_groups() {
        let mut state = MainCompleteState::new("", 0);
        let before = state.comp.groups.len();
        let _ = _first(&mut state);
        assert_eq!(state.comp.groups.len(), before);
    }
}
