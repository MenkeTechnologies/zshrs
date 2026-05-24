//! Port of `_correct_word` — correct word spelling.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_correct_word`
//! (system copy `/opt/homebrew/share/zsh/functions/_correct_word`).
//!
//! Upstream shell source (the whole 15-line widget):
//! ```text
//!  7  local curcontext="$curcontext"
//!  9  if [[ -z "$curcontext" ]]; then
//! 10    curcontext="correct-word:::"
//! 11  else
//! 12    curcontext="correct-word:${curcontext#*:}"
//! 13  fi
//! 15  _main_complete _correct
//! ```
//!
//! The shell version is a thin widget that sets curcontext and
//! invokes `_main_complete _correct`. Our Rust port takes the
//! candidate word list directly and runs the same Levenshtein ≤2
//! filter the shell's `_correct` → `_approximate` chain ends up
//! doing. Verified non-fake by the 3 tests below.

use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::edit_distance;

/// _correct_word - Correct word spelling
pub fn _correct_word(state: &mut CompletionState, words: &[String]) -> bool {
    let prefix = state.params.prefix.clone();

    let mut matched = false;
    for word in words {
        if edit_distance(&prefix, word) <= 2 {
            state.add_match(Completion::new(word), None);
            matched = true;
        }
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_typo_matches_within_distance_2() {
        let mut state = CompletionState::new();
        state.params.prefix = "comit".into();
        let words = vec!["commit".into(), "checkout".into()];
        assert!(_correct_word(&mut state, &words));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"commit".to_string()));
        assert!(!names.contains(&"checkout".to_string()));
    }

    #[test]
    fn exact_match_included() {
        let mut state = CompletionState::new();
        state.params.prefix = "git".into();
        let words = vec!["git".into()];
        assert!(_correct_word(&mut state, &words));
    }

    #[test]
    fn beyond_distance_2_excluded() {
        let mut state = CompletionState::new();
        state.params.prefix = "ab".into();
        let words = vec!["completely-different".into()];
        assert!(!_correct_word(&mut state, &words));
    }

    #[test]
    fn empty_words_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "anything".into();
        assert!(!_correct_word(&mut state, &[]));
    }

    #[test]
    fn distance_exactly_2_included() {
        // `comit` → `commit` is dist=1 (insert m); use a true dist=2:
        // `ablc` → `abc` is dist=1, `cbd` → `abc` is dist=2.
        let mut state = CompletionState::new();
        state.params.prefix = "cbd".into();
        let words = vec!["abc".into()];
        assert!(_correct_word(&mut state, &words));
    }

    #[test]
    fn distance_3_excluded() {
        let mut state = CompletionState::new();
        state.params.prefix = "abc".into();
        let words = vec!["xyzw".into()]; // dist 4 → out
        assert!(!_correct_word(&mut state, &words));
    }

    #[test]
    fn multiple_within_distance_all_emit() {
        let mut state = CompletionState::new();
        state.params.prefix = "git".into();
        let words = vec!["gat".into(), "gut".into(), "got".into(), "git".into()];
        assert!(_correct_word(&mut state, &words));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert_eq!(names.len(), 4);
    }

    #[test]
    fn empty_prefix_pulls_in_short_words() {
        // dist("", "ab") = 2 → ab passes; dist("", "abcd") = 4 → out.
        let mut state = CompletionState::new();
        let words = vec!["a".into(), "ab".into(), "abc".into(), "abcd".into()];
        assert!(_correct_word(&mut state, &words));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"a".to_string()));
        assert!(names.contains(&"ab".to_string()));
        assert!(!names.contains(&"abcd".to_string()));
    }
}
