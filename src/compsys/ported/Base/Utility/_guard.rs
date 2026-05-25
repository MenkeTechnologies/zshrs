//! Port of `_guard` from `Completion/Base/Utility/_guard`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local garbage
//! sh: 4
//! sh: 5  zparseopts -K -D -a garbage M+: J+: V+: 1 2 o+: n F: X+:
//! sh: 6
//! sh: 7  [[ "$PREFIX$SUFFIX" != $~1 ]] && return 1
//! sh: 8
//! sh: 9  shift
//! sh:10  _message -e "$*"
//! sh:11
//! sh:12  [[ -n "$PREFIX$SUFFIX" ]]
//! ```
//!
//! Upstream uses the `[[ X = $~1 ]]` pattern-match (where `$~1`
//! enables glob-expansion). If the user-typed word doesn't match
//! the guard pattern, return 1 to skip this branch. Otherwise emit
//! a descriptive `_message -e` and return based on whether anything
//! is typed.
//!
//! Simplified Rust port: returns true iff PREFIX matches the
//! supplied glob (`*`/`?`) OR (for literal patterns) starts with it.
//! Drops the `_message -e` emission + compadd opt consumption — the
//! Rust caller's downstream completer handles the user feedback.



use crate::compsys::base::MainCompleteState;

use super::shared::glob_match;

/// _guard - Guard against completing in wrong context
pub fn _guard(state: &MainCompleteState, pattern: &str) -> bool {
    // shell:7 — `[[ "$PREFIX$SUFFIX" != $~1 ]] && return 1`. The
    // match target is PREFIX+SUFFIX, not just PREFIX.
    let text = format!(
        "{}{}",
        state.comp.params.prefix, state.comp.params.suffix
    );

    if pattern.contains('*') || pattern.contains('?') {
        glob_match(pattern, &text)
    } else {
        text.starts_with(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_pattern_matches_extension() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "foo.txt".into();
        assert!(_guard(&state, "*.txt"));
        state.comp.params.prefix = "foo.rs".into();
        assert!(!_guard(&state, "*.txt"));
    }

    #[test]
    fn literal_pattern_uses_starts_with() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "git-status".into();
        assert!(_guard(&state, "git-"));
        state.comp.params.prefix = "ls".into();
        assert!(!_guard(&state, "git-"));
    }

    #[test]
    fn star_only_matches_anything_non_empty() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "anything".into();
        assert!(_guard(&state, "*"));
        state.comp.params.prefix = "".into();
        // Empty prefix → glob_matches("*", "") = true (star can match empty)
        assert!(_guard(&state, "*"));
    }

    #[test]
    fn question_mark_matches_single_char() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ab".into();
        assert!(_guard(&state, "??"));
        state.comp.params.prefix = "abc".into();
        assert!(!_guard(&state, "??"));
    }

    #[test]
    fn suffix_concatenated_to_prefix_for_check() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "git".into();
        state.comp.params.suffix = "-status".into();
        // PREFIX+SUFFIX = "git-status" → matches "git-*"
        assert!(_guard(&state, "git-*"));
    }

    #[test]
    fn empty_prefix_and_suffix_with_literal_pattern_fails() {
        let state = MainCompleteState::new("", 0);
        // empty.starts_with("foo") is false.
        assert!(!_guard(&state, "foo"));
    }

    #[test]
    fn complex_glob_with_multiple_stars() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "abc-xyz-123".into();
        assert!(_guard(&state, "*-*-*"));
        assert!(_guard(&state, "abc*123"));
        assert!(!_guard(&state, "abc*456"));
    }

    #[test]
    fn literal_pattern_must_start_exactly() {
        // starts_with is exact prefix, not substring containment.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "xfoo".into();
        assert!(!_guard(&state, "foo"), "literal must be a PREFIX");
    }

    #[test]
    fn glob_question_then_literal() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "abc".into();
        // `?bc` matches `abc` — one wild char + literal `bc`.
        assert!(_guard(&state, "?bc"));
        assert!(!_guard(&state, "?bd"));
    }
}
