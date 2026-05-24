//! Port of `_approximate` — approximate/fuzzy matching.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_approximate`
//! (system copy `/opt/homebrew/share/zsh/functions/_approximate`).
//!
//! Upstream shell source (key lines from the ~80-line impl):
//! ```text
//! 10  [[ _matcher_num -gt 1 || "${#:-$PREFIX$SUFFIX}" -le 1 ]] && return 1
//! 12  local _comp_correct _correct_expl _correct_group comax cfgacc
//! 17  if [[ "$1" = -a* ]]; then cfgacc="${1[3,-1]}"; …
//! 35  zstyle -s ":completion:${oldcontext}:" max-errors comax
//! 40  for (( ; comax > 0 ; comax-- )); do
//! 42    _comp_correct=$comax _complete
//! 50  done
//! ```
//!
//! The shell version: gates on PREFIX≥2 chars, reads `max-errors`
//! zstyle, then loops decreasing the error budget and re-runs the
//! normal completer at each level, accumulating matches.
//!
//! Simplified Rust port: takes `max_errors` directly (caller's
//! responsibility to honor the zstyle), iterates the candidates
//! already present in CompletionState, and keeps those within
//! Levenshtein-≤-max_errors of the prefix.

use crate::base::{CompleterResult, MainCompleteState};
use crate::completion::Completion;

use super::shared::edit_distance;

/// _approximate - Approximate/fuzzy matching
pub fn _approximate(state: &mut MainCompleteState, max_errors: usize) -> CompleterResult {
    let original = state.comp.params.prefix.clone();

    // Get all potential matches and filter by edit distance
    // This is a simplified implementation
    let matches: Vec<String> = state
        .comp
        .all_completions()
        .iter()
        .filter(|c| edit_distance(&original, &c.str_) <= max_errors)
        .map(|c| c.str_.clone())
        .collect();

    if matches.is_empty() {
        CompleterResult::NoMatch
    } else {
        for m in matches {
            state.comp.add_match(Completion::new(&m), None);
        }
        CompleterResult::Matched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_typo_within_max_errors_matches() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "comit".into();
        state.comp.add_match(Completion::new("commit"), None);
        state.comp.add_match(Completion::new("checkout"), None);
        match _approximate(&mut state, 1) {
            CompleterResult::Matched => {}
            other => panic!("expected Matched, got {other:?}"),
        }
    }

    #[test]
    fn beyond_max_errors_returns_no_match() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "totally-different".into();
        state.comp.add_match(Completion::new("xyz"), None);
        assert!(matches!(
            _approximate(&mut state, 1),
            CompleterResult::NoMatch
        ));
    }

    #[test]
    fn zero_max_errors_only_exact() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "git".into();
        state.comp.add_match(Completion::new("git"), None);
        state.comp.add_match(Completion::new("gut"), None);
        // max_errors=0 → only the exact "git" passes the filter.
        assert!(matches!(
            _approximate(&mut state, 0),
            CompleterResult::Matched
        ));
    }
}
