//! Port of `_ignored` — complete previously ignored matches.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_ignored`
//! (system copy `/opt/homebrew/share/zsh/functions/_ignored`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  5  [[ _matcher_num -gt 1 || $compstate[ignored] -eq 0 ]] && return 1
//!  7  local comp
//!  9  if ! zstyle -a ":completion:${curcontext}:" completer comp; then
//! 10    comp=( "${(@)_completers[1,_completer_num-1]}" )
//! 11    ind=${comp[(I)_ignored(|:*)]}
//! 14  local _comp_no_ignore=yes …
//! ```
//!
//! The shell version re-runs the preceding completers with
//! `_comp_no_ignore=yes` set so they emit the matches that had been
//! filtered out by `ignored-patterns` zstyle.
//!
//! Strict Rust port of the GATE half of the shell function. The
//! gating is the only piece the leaf layer can implement here:
//! re-running prior completers under `_comp_no_ignore=yes` is the
//! caller's job (it owns the completer dispatch loop). Returns
//! true iff the caller SHOULD run that loop now.
//!
//! Gate semantics (shell:5):
//!   `[[ _matcher_num -gt 1 || $compstate[ignored] -eq 0 ]] && return 1`
//! → return false (don't run) when either we're past matcher 1 OR
//!   there are no ignored matches to recover. Otherwise return true.

use crate::compcore::CompletionState;

/// _ignored - Complete previously ignored matches.
///
/// `matcher_num` mirrors the shell variable `_matcher_num` (1-based
/// index of the current matcher within the matcher-list). `state.ignored`
/// mirrors `$compstate[ignored]` (count of previously-suppressed
/// matches). `ignored_patterns` is the resolved `ignored-patterns`
/// zstyle list — currently consumed only as a gate, but kept in the
/// signature so callers wire the real value (not yet applied because
/// the re-run happens in the caller).
pub fn _ignored(
    state: &mut CompletionState,
    matcher_num: usize,
    ignored_patterns: &[String],
) -> bool {
    let _ = ignored_patterns;
    // shell:5 — past first matcher OR no ignored count → bail.
    if matcher_num > 1 || state.ignored == 0 {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_matcher_with_ignored_matches_returns_true() {
        let mut state = CompletionState::new();
        state.ignored = 3;
        assert!(_ignored(&mut state, 1, &[]));
    }

    #[test]
    fn second_matcher_bails_even_with_ignored() {
        // shell:5 — `_matcher_num -gt 1` short-circuits to false
        // regardless of how many ignored matches there are.
        let mut state = CompletionState::new();
        state.ignored = 99;
        assert!(!_ignored(&mut state, 2, &[]));
    }

    #[test]
    fn zero_ignored_means_false() {
        let mut state = CompletionState::new();
        assert!(!_ignored(&mut state, 1, &["pat".into()]));
    }

    #[test]
    fn patterns_arg_does_not_short_circuit_gate() {
        let mut state = CompletionState::new();
        state.ignored = 5;
        assert!(_ignored(&mut state, 1, &[]));
        assert!(_ignored(&mut state, 1, &["a".into()]));
        assert!(_ignored(
            &mut state,
            1,
            &["a".into(), "b".into(), "c".into()]
        ));
    }

    #[test]
    fn matcher_num_zero_treated_as_first_pass() {
        // Some callers index from 0; matcher_num=0 is NOT > 1, so it
        // still passes the gate.
        let mut state = CompletionState::new();
        state.ignored = 1;
        assert!(_ignored(&mut state, 0, &[]));
    }

    #[test]
    fn high_ignored_count_first_matcher_still_runs() {
        let mut state = CompletionState::new();
        state.ignored = 999_999;
        assert!(_ignored(&mut state, 1, &[]));
    }

    #[test]
    fn zero_ignored_with_zero_matcher_still_false() {
        // Both gates together — both 0 → still bails (the ignored=0
        // gate).
        let mut state = CompletionState::new();
        assert!(!_ignored(&mut state, 0, &[]));
    }

    #[test]
    fn matcher_exactly_1_does_not_trip_gt_1_gate() {
        // The shell condition is `_matcher_num -gt 1` — strict >, so
        // matcher_num==1 should NOT trigger the bail.
        let mut state = CompletionState::new();
        state.ignored = 5;
        assert!(_ignored(&mut state, 1, &[]));
    }

    #[test]
    fn does_not_mutate_state_or_emit() {
        let mut state = CompletionState::new();
        state.ignored = 3;
        let before_groups = state.groups.len();
        let before_n = state.nmatches;
        _ignored(&mut state, 1, &["pat".into()]);
        assert_eq!(state.groups.len(), before_groups);
        assert_eq!(state.nmatches, before_n);
        // ignored count should stay the same — _ignored is a gate.
        assert_eq!(state.ignored, 3);
    }
}
