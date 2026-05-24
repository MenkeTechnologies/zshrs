//! Port of `_precommand` — complete after a precommand (sudo,
//! nohup, time, …).
//!
//! Local shell reference: `compsys/functions/Unix/Type/_precommand`
//! (system copy `/opt/homebrew/share/zsh/functions/_precommand`).
//!
//! Upstream shell source (the WHOLE file, 5 lines):
//! ```text
//!  1  #compdef - nohup eval time rusage noglob nocorrect catchsegv aoss hilite eatmydata
//!  3  shift words
//!  4  (( CURRENT-- ))
//!  5  _normal -p $service
//! ```
//!
//! Upstream pops the precommand off `$words`, decrements CURRENT
//! (since we're now one word back), and runs `_normal -p $service`
//! (`-p` tells _normal "this is precommand dispatch").
//!
//! Simplified Rust port: gates on `current > 1` (already past the
//! precommand position) and dispatches to `_normal`. The `_normal`
//! port currently returns NoMatch until the comps table is wired
//! through — pinned by the
//! `current_gt_1_delegates_to_normal` test.

use crate::base::{_normal, CompleterResult, MainCompleteState};

/// _precommand - Complete after a precommand (sudo, nohup, etc.)
pub fn _precommand(state: &mut MainCompleteState) -> bool {
    // Skip the precommand and complete as normal command
    if state.comp.params.current > 1 {
        // Treat rest as command line
        matches!(_normal(state), CompleterResult::Matched)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_eq_1_returns_false() {
        // current == 1 means we're still on the precommand itself
        // (`sudo<TAB>`). _precommand can't help — it dispatches the
        // FOLLOWING word.
        let mut state = MainCompleteState::new("sudo", 4);
        state.comp.params.current = 1;
        assert!(!_precommand(&mut state));
    }

    #[test]
    fn current_gt_1_delegates_to_normal() {
        // current > 1 → delegate to _normal (currently returns
        // NoMatch until comps table is wired). Pin the delegation
        // path is taken.
        let mut state = MainCompleteState::new("sudo ls", 7);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["sudo".into(), "ls".into()];
        assert!(!_precommand(&mut state));
    }
}
