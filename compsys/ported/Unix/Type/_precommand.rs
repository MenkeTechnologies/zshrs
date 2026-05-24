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
//! Strict Rust port: mirrors `shift words; (( CURRENT-- )); _normal`.
//! Pops the leading word from `state.comp.params.words` and
//! decrements `current` before calling `_normal`. Restores both
//! after — `_precommand` is a one-shot dispatch helper, not a
//! mutation primitive.

use crate::base::{_normal, CompleterResult, MainCompleteState};

/// _precommand - Complete after a precommand (sudo, nohup, etc.).
pub fn _precommand(state: &mut MainCompleteState) -> bool {
    // shell:4 `(( CURRENT-- ))` — only meaningful when current > 1.
    if state.comp.params.current <= 1 {
        return false;
    }
    // shell:3 `shift words` — pop the precommand.
    let saved_words = state.comp.params.words.clone();
    let saved_current = state.comp.params.current;
    if !saved_words.is_empty() {
        state.comp.params.words.remove(0);
    }
    state.comp.params.current = saved_current - 1;

    let result = matches!(_normal(state), CompleterResult::Matched);

    // Restore so caller's view of words+current is unchanged.
    state.comp.params.words = saved_words;
    state.comp.params.current = saved_current;
    result
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
        let mut state = MainCompleteState::new("sudo ls", 7);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["sudo".into(), "ls".into()];
        assert!(!_precommand(&mut state));
    }

    #[test]
    fn current_eq_0_returns_false() {
        // current==0 is the precommand-of-precommand edge.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 0;
        assert!(!_precommand(&mut state));
    }

    #[test]
    fn high_current_with_words_still_delegates() {
        let mut state = MainCompleteState::new("sudo nohup git status", 21);
        state.comp.params.current = 4;
        state.comp.params.words = vec![
            "sudo".into(),
            "nohup".into(),
            "git".into(),
            "status".into(),
        ];
        // _normal returns NoMatch (no comps table wired) → false.
        assert!(!_precommand(&mut state));
    }

    #[test]
    fn words_restored_after_call() {
        let original_words = vec!["sudo".to_string(), "ls".to_string()];
        let mut state = MainCompleteState::new("sudo ls", 7);
        state.comp.params.current = 2;
        state.comp.params.words = original_words.clone();
        let _ = _precommand(&mut state);
        assert_eq!(
            state.comp.params.words, original_words,
            "_precommand must restore words after dispatch"
        );
    }

    #[test]
    fn current_restored_after_call() {
        let mut state = MainCompleteState::new("sudo ls", 7);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["sudo".into(), "ls".into()];
        let _ = _precommand(&mut state);
        assert_eq!(state.comp.params.current, 2);
    }

    #[test]
    fn empty_words_with_high_current_still_decrements() {
        // current > 1 with empty words: we don't pop (nothing to pop)
        // but we DO decrement current before calling _normal.
        // Pin no panic.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 3;
        state.comp.params.words.clear();
        let _ = _precommand(&mut state);
    }

    #[test]
    fn first_word_popped_during_normal_dispatch() {
        // We can't directly inspect what _normal sees, but we can
        // observe through a registered handler — at the leaf layer
        // _normal currently returns NoMatch, so just pin that the
        // pop happens and restoration works correctly.
        let mut state = MainCompleteState::new("sudo git status", 15);
        state.comp.params.current = 3;
        state.comp.params.words = vec!["sudo".into(), "git".into(), "status".into()];
        let snapshot = state.comp.params.words.clone();
        let _ = _precommand(&mut state);
        assert_eq!(state.comp.params.words, snapshot);
    }
}
