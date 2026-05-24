//! Port of `_history_complete_word` — complete word from history.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_history_complete_word`
//! (system copy `/opt/homebrew/share/zsh/functions/_history_complete_word`).
//!
//! Upstream shell source (header — full impl is ~100 lines):
//! ```text
//!  1  #compdef -K _history-complete-older complete-word \e/ \
//!                _history-complete-newer complete-word \e,
//! 17  _history_complete_word () {
//! 18    eval "$_comp_setup"
//! 20    local expl direction stop curcontext="$curcontext"
//! ```
//!
//! The upstream version honors styles like `range`, `sort`, `stop`,
//! `list`, `remove-all-dups` and walks the global $history array
//! with cycling/wrap semantics.
//!
//! Strict Rust port: takes the history array directly. Honors
//! `remove-all-dups` (dedup all matches), `sort` (lexical sort
//! before emit), and `stop` (return false after first match,
//! single-shot behavior). Walks forward or backward by direction.

use std::collections::HashSet;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// Options mirroring the `range` / `sort` / `stop` / `remove-all-dups`
/// zstyles upstream consults.
#[derive(Default)]
pub struct HistoryCompleteOpts {
    /// `direction` — -1 = backward (default; matches Esc-/ binding),
    /// 1 = forward (Esc-, binding).
    pub direction: i32,
    /// `remove-all-dups` zstyle — drop duplicate words.
    pub remove_all_dups: bool,
    /// `sort` zstyle — emit alphabetically sorted matches.
    pub sort: bool,
    /// `stop` zstyle — return after first match (the "stop iterating"
    /// branch). Default false: collect ALL matches.
    pub stop: bool,
    /// Cap on number of matches emitted (0 = unlimited).
    pub max_matches: usize,
}

/// _history_complete_word - Complete word from history.
pub fn _history_complete_word(
    state: &mut CompletionState,
    history_entries: &[String],
    opts: &HistoryCompleteOpts,
) -> bool {
    let prefix = state.params.prefix.clone();
    let iter: Box<dyn Iterator<Item = &String>> = if opts.direction < 0 {
        Box::new(history_entries.iter().rev())
    } else {
        Box::new(history_entries.iter())
    };

    let mut collected: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    'outer: for entry in iter {
        for word in entry.split_whitespace() {
            if word.starts_with(&prefix) && word != prefix {
                if opts.remove_all_dups {
                    if !seen.insert(word.to_string()) {
                        continue;
                    }
                }
                collected.push(word.to_string());
                if opts.stop {
                    break 'outer;
                }
                if opts.max_matches > 0 && collected.len() >= opts.max_matches {
                    break 'outer;
                }
            }
        }
    }

    if collected.is_empty() {
        return false;
    }

    if opts.sort {
        collected.sort();
    }
    for w in collected {
        state.add_match(Completion::new(&w), None);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(direction: i32) -> HistoryCompleteOpts {
        HistoryCompleteOpts {
            direction,
            ..Default::default()
        }
    }

    #[test]
    fn forward_iter_finds_match_after_prefix() {
        let mut state = CompletionState::new();
        state.params.prefix = "che".into();
        let history = vec!["ls -la".into(), "git checkout main".into()];
        assert!(_history_complete_word(&mut state, &history, &opts(1)));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"checkout".to_string()));
    }

    #[test]
    fn backward_iter_walks_recent_first() {
        let mut state = CompletionState::new();
        state.params.prefix = "che".into();
        let history = vec![
            "git checkout old".into(),
            "git cherry-pick xyz".into(),
        ];
        // direction=-1 + stop=true → first match seen is from the
        // newest entry (cherry-pick).
        let mut o = opts(-1);
        o.stop = true;
        assert!(_history_complete_word(&mut state, &history, &o));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert_eq!(names, vec!["cherry-pick"]);
    }

    #[test]
    fn word_equal_to_prefix_skipped() {
        let mut state = CompletionState::new();
        state.params.prefix = "exactly".into();
        let history = vec!["exactly".into()];
        assert!(!_history_complete_word(&mut state, &history, &opts(1)));
    }

    #[test]
    fn collects_all_matches_by_default() {
        let mut state = CompletionState::new();
        state.params.prefix = "c".into();
        let history = vec!["cat".into(), "checkout".into(), "cherry-pick".into()];
        assert!(_history_complete_word(&mut state, &history, &opts(1)));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn remove_all_dups_deduplicates() {
        let mut state = CompletionState::new();
        state.params.prefix = "g".into();
        let history = vec![
            "git status".into(),
            "git status".into(),
            "git diff".into(),
            "git diff".into(),
        ];
        let mut o = opts(1);
        o.remove_all_dups = true;
        let _ = _history_complete_word(&mut state, &history, &o);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert_eq!(names.len(), 1, "only `git` survives dedup");
        assert_eq!(names[0], "git");
    }

    #[test]
    fn sort_orders_emitted_matches_lexically() {
        let mut state = CompletionState::new();
        state.params.prefix = "c".into();
        let history = vec!["cat zoo".into(), "cd zoo".into(), "cherry zoo".into()];
        let mut o = opts(1);
        o.sort = true;
        o.remove_all_dups = true;
        let _ = _history_complete_word(&mut state, &history, &o);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn max_matches_caps_collection() {
        let mut state = CompletionState::new();
        state.params.prefix = "c".into();
        let history = vec![
            "cat".into(),
            "checkout".into(),
            "cherry-pick".into(),
            "commit".into(),
        ];
        let mut o = opts(1);
        o.max_matches = 2;
        let _ = _history_complete_word(&mut state, &history, &o);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn empty_history_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "anything".into();
        assert!(!_history_complete_word(&mut state, &[], &opts(1)));
    }
}
