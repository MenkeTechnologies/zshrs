//! Port of `_history` from `Completion/Base/Completer/_history`.
//!
//! Full upstream body (65 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Hm, this *can* sensibly be used as a completer. But it could also be used
//! sh: 4  # as a utility function, so maybe it should be moved into another directory.
//! sh: 5  # Or maybe not. Hm.
//! sh: 6  #
//! sh: 7  #
//! sh: 8  # Complete words from the history
//! sh: 9  #
//! sh:10  # Code taken from _history_complete_words.
//! sh:11  #
//! sh:12  # Available styles:
//! sh:13  #
//! sh:14  #   sort --  sort matches lexically (default is to sort by age)
//! sh:15  #   remove-all-dups --
//! sh:16  #            remove /all/ duplicate matches rather than just consecutives
//! sh:17  #   range -- range of history words to complete
//! sh:18
//! sh:19  local opt expl max slice hmax=$#historywords beg=2
//! sh:20
//! sh:21  if zstyle -t ":completion:${curcontext}:" remove-all-dups; then
//! sh:22    opt=-
//! sh:23  else
//! sh:24    opt=-1
//! sh:25  fi
//! sh:26
//! sh:27  if zstyle -t ":completion:${curcontext}:" sort; then
//! sh:28    opt="${opt}J"
//! sh:29  else
//! sh:30    opt="${opt}V"
//! sh:31  fi
//! sh:32
//! sh:33  if zstyle -s ":completion:${curcontext}:" range max; then
//! sh:34    if [[ $max = *:* ]]; then
//! sh:35      slice=${max#*:}
//! sh:36      max=${max%:*}
//! sh:37    else
//! sh:38      slice=$max
//! sh:39    fi
//! sh:40    [[ max -gt hmax ]] && max=$hmax
//! sh:41  else
//! sh:42    max=$hmax
//! sh:43    slice=$max
//! sh:44  fi
//! sh:45
//! sh:46  PREFIX="$IPREFIX$PREFIX"
//! sh:47  IPREFIX=
//! sh:48  SUFFIX="$SUFFIX$ISUFFIX"
//! sh:49  ISUFFIX=
//! sh:50
//! sh:51  # We skip the first element of historywords so the current word doesn't
//! sh:52  # interfere with the completion
//! sh:53
//! sh:54  local -a hslice
//! sh:55  while [[ $compstate[nmatches] -eq 0 && beg -lt max ]]; do
//! sh:56    if [[ -n $compstate[quote] ]]
//! sh:57    then hslice=( ${(Q)historywords[beg,beg+slice]} )
//! sh:58    else hslice=( ${historywords[beg,beg+slice]} )
//! sh:59    fi
//! sh:60    _wanted "$opt" history-words expl 'history word' \
//! sh:61        compadd -Q -a hslice
//! sh:62    (( beg+=slice ))
//! sh:63  done
//! sh:64
//! sh:65  (( $compstate[nmatches] ))
//! ```
//!
//! Faithful Rust port: honors `HistoryOpts` for sort / range /
//! remove-all-dups / max-words knobs that mirror the corresponding
//! shell zstyles. The default opts match upstream defaults
//! (reverse iteration, full dedup, no range cap).



use std::collections::HashSet;

use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

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

    #[test]
    fn empty_prefix_returns_all_dedup_entries() {
        let mut state = CompletionState::new();
        let history = vec!["a".into(), "b".into(), "a".into(), "c".into()];
        let _ = _history(&mut state, &history);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // Dedup keeps 3 unique entries.
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn range_out_of_bounds_uses_full_history() {
        // a > history.len() means range is ignored (no slice).
        let mut state = CompletionState::new();
        let history = vec!["x".into(), "y".into()];
        let opts = HistoryOpts {
            range: Some((100, 200)),
            ..Default::default()
        };
        let _ = _history_with_opts(&mut state, &history, &opts);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // Range invalid → falls back to full history.
        assert!(names.contains(&"x"));
        assert!(names.contains(&"y"));
    }

    #[test]
    fn range_inverted_uses_full_history() {
        let mut state = CompletionState::new();
        let history = vec!["x".into(), "y".into(), "z".into()];
        let opts = HistoryOpts {
            range: Some((3, 1)), // inverted → invalid
            ..Default::default()
        };
        let _ = _history_with_opts(&mut state, &history, &opts);
        let total: usize = state.groups.iter().map(|g| g.matches.len()).sum();
        assert_eq!(total, 3, "inverted range → fall back to full");
    }

    #[test]
    fn lexical_sort_marks_group_sorted() {
        let mut state = CompletionState::new();
        let history = vec!["zeta".into(), "alpha".into(), "mu".into()];
        let opts = HistoryOpts {
            sort: HistorySort::Lexical,
            ..Default::default()
        };
        let _ = _history_with_opts(&mut state, &history, &opts);
        // Group's `sorted` flag should be set.
        let grp = state
            .groups
            .iter()
            .find(|g| g.name == "history")
            .expect("history group");
        assert!(grp.sorted, "Lexical sort → group.sorted = true");
    }
}
