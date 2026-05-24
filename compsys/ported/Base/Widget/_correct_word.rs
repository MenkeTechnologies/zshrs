//! Port of `_correct_word` — correct word spelling. Moved from
//! `compsys/functions.rs`.

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
}
