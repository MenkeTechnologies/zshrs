//! Port of `_guard` — guard against completing in wrong context. Moved
//! from `compsys/functions.rs`.

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
