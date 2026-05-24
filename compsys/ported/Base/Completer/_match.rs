//! Port of `_match` — pattern-based matching.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_match`
//! (system copy `/opt/homebrew/share/zsh/functions/_match`).
//!
//! Upstream shell source (comment-only summary at the top):
//! ```text
//!  4  # Intended to be used as a completer function after the normal
//!  5  # completer as in:
//!  6  #   zstyle ":completion:::::" completer _complete _match
//!  9  # Note that this is only really useful if you don't use the
//! 10  # expand-or-complete function because otherwise the pattern
//! 11  # will be expanded using globbing.
//! ```
//!
//! Upstream flips `compstate[pattern_match]='*'` and re-runs the
//! previous completers so they accept glob-pattern input (user types
//! `*.rs<TAB>` and gets matches the literal-prefix completer
//! wouldn't produce).
//!
//! Simplified Rust port: takes the pattern + candidate list directly
//! and emits candidates that glob-match. Supports `*` and `?`.

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

    #[test]
    fn no_matches_returns_false_without_emitting() {
        let mut state = CompletionState::new();
        let cands = vec!["alpha".into(), "beta".into()];
        assert!(!_match(&mut state, "*.rs", &cands));
        assert_eq!(state.nmatches, 0);
    }

    #[test]
    fn anchored_pattern_matches_full_string() {
        // glob_match is anchored — `foo*` matches `foobar` but not
        // `xfoobar`.
        let mut state = CompletionState::new();
        let cands = vec!["foobar".into(), "xfoobar".into(), "foo".into()];
        assert!(_match(&mut state, "foo*", &cands));
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"foobar"));
        assert!(names.contains(&"foo"));
        assert!(!names.contains(&"xfoobar"), "anchor must hold");
    }

    #[test]
    fn literal_pattern_matches_exact_candidate_only() {
        let mut state = CompletionState::new();
        let cands = vec!["abc".into(), "abcd".into(), "ab".into()];
        assert!(_match(&mut state, "abc", &cands));
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["abc"]);
    }

    #[test]
    fn star_alone_matches_everything() {
        let mut state = CompletionState::new();
        let cands = vec!["".into(), "a".into(), "longer-thing".into()];
        assert!(_match(&mut state, "*", &cands));
        let count: usize = state.groups.iter().map(|g| g.matches.len()).sum();
        assert_eq!(count, cands.len());
    }

    #[test]
    fn multiple_globs_in_pattern() {
        let mut state = CompletionState::new();
        let cands = vec![
            "a_b_c".into(),
            "axx_byy_czz".into(),
            "a_b".into(),
            "x_b_c".into(),
        ];
        assert!(_match(&mut state, "a*_b*_c*", &cands));
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"a_b_c"));
        assert!(names.contains(&"axx_byy_czz"));
        assert!(!names.contains(&"a_b"));
        assert!(!names.contains(&"x_b_c"));
    }
}
