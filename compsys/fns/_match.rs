//! Port of `_match` — pattern-based matching. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::glob_match;

/// _match - Pattern-based matching
pub fn _match(state: &mut CompletionState, pattern: &str, candidates: &[String]) -> bool {
    let mut matched = false;

    for candidate in candidates {
        if glob_match(pattern, candidate) {
            state.add_match(Completion::new(candidate), None);
            matched = true;
        }
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_glob_matches_substring() {
        let mut state = CompletionState::new();
        let candidates = vec!["foo.rs".into(), "bar.txt".into(), "baz.rs".into()];
        let ok = _match(&mut state, "*.rs", &candidates);
        assert!(ok);
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"foo.rs"));
        assert!(names.contains(&"baz.rs"));
        assert!(!names.contains(&"bar.txt"), ".txt should not match *.rs");
    }

    #[test]
    fn question_mark_glob_matches_single_char() {
        let mut state = CompletionState::new();
        let candidates = vec!["ab".into(), "abc".into(), "ad".into()];
        let ok = _match(&mut state, "a?", &candidates);
        assert!(ok);
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"ab"));
        assert!(names.contains(&"ad"));
        assert!(!names.contains(&"abc"), "? should match exactly one char");
    }

    #[test]
    fn empty_candidates_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_match(&mut state, "*", &[]));
    }
}
