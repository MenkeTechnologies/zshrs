//! Port of `_parameters` — complete parameter (variable) names.
//!
//! Local shell reference: `compsys/functions/Zsh/Type/_parameters`
//! (system copy `/opt/homebrew/share/zsh/functions/_parameters`).
//!
//! Upstream shell source (key lines from ~30-line fn):
//! ```text
//!  9  if compset -P '*:'; then
//! 10    _history_modifiers p
//! 11    return
//! 12  fi
//! 18  zparseopts -E -a opts g:=pfilt
//! 21  if (( $#pfilt )); then
//! 22    i=( ${(k)parameters[(R)$pfilt[2]]} )
//! 26    i=( ${(k)parameters} )
//! ```
//!
//! Upstream pulls names from `${(k)parameters}` (built-in assoc
//! array mapping name→type) with optional `-g pattern` type filter.
//!
//! Simplified Rust port: takes a `&HashMap<String, String>` from
//! the caller (caller pulls live names from runtime paramtab) and
//! emits names prefix-filtered. The `-g` type filter is not yet
//! exposed — most call sites want every parameter name.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _parameters - Complete parameter (variable) names
pub fn _parameters(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("parameters", true);

    for name in params.keys() {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("parameters"));
        }
    }

    state.end_group();
    state.nmatches > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_only_prefix_matching_keys() {
        let mut state = CompletionState::new();
        state.params.prefix = "HOM".into();
        let mut params = HashMap::new();
        params.insert("HOME".into(), "/root".into());
        params.insert("HOST".into(), "x".into());
        params.insert("USER".into(), "wizard".into());
        let ok = _parameters(&mut state, &params);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["HOME"], "HOST also starts with HO but not HOM; USER not in prefix");
    }

    #[test]
    fn empty_prefix_emits_all_keys() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("A".into(), "1".into());
        params.insert("B".into(), "2".into());
        let ok = _parameters(&mut state, &params);
        assert!(ok);
        assert_eq!(state.groups[0].matches.len(), 2);
    }

    #[test]
    fn returns_false_when_no_matches() {
        let mut state = CompletionState::new();
        state.params.prefix = "ZZZ".into();
        let mut params = HashMap::new();
        params.insert("HOME".into(), "/root".into());
        assert!(!_parameters(&mut state, &params));
    }
}
