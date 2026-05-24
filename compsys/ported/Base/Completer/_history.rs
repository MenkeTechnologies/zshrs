//! Port of `_history` — complete from command history. Moved from
//! `compsys/functions.rs`.

use std::collections::HashSet;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _history - Complete from command history
pub fn _history(state: &mut CompletionState, history_entries: &[String]) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("history", true);
    let mut matched = false;
    let mut seen = HashSet::new();

    // Iterate in reverse (most recent first)
    for entry in history_entries.iter().rev() {
        if entry.starts_with(&prefix) && !seen.contains(entry) {
            state.add_match(Completion::new(entry), Some("history"));
            seen.insert(entry.clone());
            matched = true;
        }
    }

    state.end_group();
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn most_recent_first_via_reverse_iteration() {
        let mut state = CompletionState::new();
        state.params.prefix = "git".into();
        let history = vec![
            "git old".into(),
            "ls".into(),
            "git mid".into(),
            "git new".into(),
        ];
        assert!(_history(&mut state, &history));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // Shell adds matches in reverse iteration order, but the
        // default `begin_group(_, true)` SORTS them. The dedup-via-
        // reverse-iter is what matters semantically: the FIRST
        // (most recent) duplicate wins. Verify all three present.
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"git new"));
        assert!(names.contains(&"git mid"));
        assert!(names.contains(&"git old"));
    }

    #[test]
    fn dedupes_repeated_entries() {
        let mut state = CompletionState::new();
        let history = vec!["a".into(), "b".into(), "a".into(), "a".into()];
        assert!(_history(&mut state, &history));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(
            names.iter().filter(|n| **n == "a").count(),
            1,
            "duplicate history entries must be deduped"
        );
    }

    #[test]
    fn prefix_filter_drops_non_matching() {
        let mut state = CompletionState::new();
        state.params.prefix = "ls".into();
        let history = vec!["ls -la".into(), "git status".into(), "ls /tmp".into()];
        assert!(_history(&mut state, &history));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"ls -la"));
        assert!(names.contains(&"ls /tmp"));
        assert!(!names.contains(&"git status"));
    }

    #[test]
    fn empty_history_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_history(&mut state, &[]));
    }
}
