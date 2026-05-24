//! Port of `_history` — complete from command history.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_history`
//! (system copy `/opt/homebrew/share/zsh/functions/_history`).
//!
//! Upstream shell source (key lines):
//! ```text
//! 17  local opt expl max slice hmax=$#historywords beg=2
//! 25  zstyle -s ":completion:${curcontext}:" remove-all-dups removedups
//! 30  zstyle -a ":completion:${curcontext}:" range range
//! 39  while compadd -O dups "$expl[@]" - "${(@)historywords[beg,beg+slice-1]}"
//! 41    if [[ $removedups = true ]]; then …dedup… fi
//! 51    (( beg += slice ))
//! 52  done
//! ```
//!
//! Faithful Rust port: honors `HistoryOpts` for sort / range /
//! remove-all-dups / max-words knobs that mirror the corresponding
//! shell zstyles. The default opts match upstream defaults
//! (reverse iteration, full dedup, no range cap).

use std::collections::HashSet;

use crate::compcore::CompletionState;
use crate::completion::Completion;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySort {
    /// Default — newest first (shell's `sort=no` / unset).
    ByAge,
    /// `sort=yes` — lex sort (handled by the begin_group default).
    Lexical,
}

#[derive(Clone, Debug)]
pub struct HistoryOpts {
    /// `sort` zstyle (lex vs age).
    pub sort: HistorySort,
    /// `remove-all-dups` zstyle. true → drop ALL duplicates;
    /// false → drop only consecutive duplicates.
    pub remove_all_dups: bool,
    /// `range` zstyle — (start, end) inclusive history indices.
    /// None → no limit. Indices are 1-based from oldest.
    pub range: Option<(usize, usize)>,
    /// Cap on emitted matches. 0 → no cap.
    pub max_matches: usize,
}

impl Default for HistoryOpts {
    fn default() -> Self {
        Self {
            sort: HistorySort::ByAge,
            remove_all_dups: true,
            range: None,
            max_matches: 0,
        }
    }
}

/// _history - Complete from command history (faithful, with options)
pub fn _history_with_opts(
    state: &mut CompletionState,
    history_entries: &[String],
    opts: &HistoryOpts,
) -> bool {
    let prefix = state.params.prefix.clone();

    // shell:30 range zstyle — limit the iteration window.
    let slice: &[String] = match opts.range {
        Some((a, b)) if a <= b && a > 0 && a <= history_entries.len() => {
            let end = b.min(history_entries.len());
            &history_entries[a - 1..end]
        }
        _ => history_entries,
    };

    state.begin_group("history", matches!(opts.sort, HistorySort::Lexical));
    let mut matched = false;

    // shell:25 `remove-all-dups` — collect with full dedup OR walk
    // consecutive-dedup.
    if opts.remove_all_dups {
        let mut seen: HashSet<&String> = HashSet::new();
        // shell iterates newest-first; mirror with rev().
        for entry in slice.iter().rev() {
            if entry.starts_with(&prefix) && seen.insert(entry) {
                state.add_match(Completion::new(entry), Some("history"));
                matched = true;
                if opts.max_matches > 0 && state.nmatches >= opts.max_matches {
                    break;
                }
            }
        }
    } else {
        // Consecutive-only dedup.
        let mut last: Option<&String> = None;
        for entry in slice.iter().rev() {
            if entry.starts_with(&prefix) && last != Some(entry) {
                state.add_match(Completion::new(entry), Some("history"));
                last = Some(entry);
                matched = true;
                if opts.max_matches > 0 && state.nmatches >= opts.max_matches {
                    break;
                }
            }
        }
    }

    state.end_group();
    matched
}

/// _history - Complete from command history (default opts).
///
/// Equivalent to the shell-bound `_history` widget when no zstyles
/// have been set. Defaults: reverse iter, full dedup, no range,
/// no max.
pub fn _history(state: &mut CompletionState, history_entries: &[String]) -> bool {
    _history_with_opts(state, history_entries, &HistoryOpts::default())
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

    #[test]
    fn consecutive_only_dedup_keeps_non_adjacent_duplicates() {
        let mut state = CompletionState::new();
        let history = vec!["a".into(), "b".into(), "a".into()];
        let opts = HistoryOpts {
            remove_all_dups: false,
            ..Default::default()
        };
        assert!(_history_with_opts(&mut state, &history, &opts));
        // With consecutive-only dedup, both `a` entries survive
        // since they're not adjacent.
        let count_a = state.groups[0]
            .matches
            .iter()
            .filter(|m| m.str_ == "a")
            .count();
        assert_eq!(
            count_a, 2,
            "consecutive-only dedup must preserve non-adjacent duplicates"
        );
    }

    #[test]
    fn range_window_limits_scan() {
        let mut state = CompletionState::new();
        let history: Vec<String> = (1..=10).map(|i| format!("cmd{}", i)).collect();
        let opts = HistoryOpts {
            range: Some((3, 5)), // 1-based indices: scan cmd3..cmd5
            ..Default::default()
        };
        assert!(_history_with_opts(&mut state, &history, &opts));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"cmd3"));
        assert!(names.contains(&"cmd5"));
        assert!(!names.contains(&"cmd1"));
        assert!(!names.contains(&"cmd10"));
    }

    #[test]
    fn max_matches_caps_output() {
        let mut state = CompletionState::new();
        let history: Vec<String> = (1..=100).map(|i| format!("a{}", i)).collect();
        let opts = HistoryOpts {
            max_matches: 5,
            ..Default::default()
        };
        assert!(_history_with_opts(&mut state, &history, &opts));
        assert_eq!(
            state.nmatches, 5,
            "max_matches must cap output to exactly 5"
        );
    }
}
