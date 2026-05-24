//! Port of `_guard` — guard against completing in wrong context.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_guard`
//! (system copy `/opt/homebrew/share/zsh/functions/_guard`).
//!
//! Upstream shell source (the whole 12-line fn):
//! ```text
//!  3  local garbage
//!  5  zparseopts -K -D -a garbage M+: J+: V+: 1 2 o+: n F: X+:
//!  7  [[ "$PREFIX$SUFFIX" != $~1 ]] && return 1
//!  9  shift
//! 10  _message -e "$*"
//! 12  [[ -n "$PREFIX$SUFFIX" ]]
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

use crate::base::MainCompleteState;

use super::shared::glob_match;

/// _guard - Guard against completing in wrong context
pub fn _guard(state: &MainCompleteState, pattern: &str) -> bool {
    let prefix = state.comp.params.prefix.clone();

    // Simple glob matching
    if pattern.contains('*') || pattern.contains('?') {
        glob_match(pattern, &prefix)
    } else {
        prefix.starts_with(pattern)
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
}
