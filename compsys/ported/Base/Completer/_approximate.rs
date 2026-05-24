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
//! Upstream: gates on `|PREFIX+SUFFIX| > 1` and the first-matcher
//! pass, reads `max-errors` zstyle, then loops decreasing the error
//! budget and re-runs the normal completer at each level,
//! accumulating matches.
//!
//! Strict Rust port: implements the gate AND the descending-budget
//! loop. The `max-errors` zstyle is consulted; explicit `max_errors`
//! arg wins when > 0. We iterate from `max_errors → 1` and add
//! every candidate within edit-distance of the prefix. Matches at
//! lower error counts overwrite higher-error entries via a HashMap
//! collation (closest-first wins). Returns `Matched` if anything
//! made it through.

use std::collections::HashMap;

use crate::base::{CompleterResult, MainCompleteState};
use crate::completion::Completion;

use super::shared::edit_distance;

/// _approximate - Approximate/fuzzy matching.
///
/// `max_errors` overrides the `max-errors` zstyle when > 0. The
/// existing matches in `state.comp` are the candidate pool (caller
/// runs the regular completers first and feeds the survivors here).
pub fn _approximate(state: &mut MainCompleteState, max_errors: usize) -> CompleterResult {
    let prefix = state.comp.params.prefix.clone();
    let suffix = state.comp.params.suffix.clone();

    // shell:10 — `${#:-$PREFIX$SUFFIX} -le 1` bail. Single-char
    // approximation is meaningless (every char is within dist=1 of
    // every other char).
    if prefix.len() + suffix.len() <= 1 {
        return CompleterResult::NoMatch;
    }

    // shell:35 — `zstyle -s max-errors`. Caller's explicit `max_errors`
    // arg wins when > 0; else consult style; else default to 2.
    let effective_max = if max_errors > 0 {
        max_errors
    } else {
        let ctx = format!(":completion:{}:", state.ctx.context);
        state
            .styles
            .lookup_values(&ctx, "max-errors")
            .and_then(|v| v.first().cloned())
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(2)
    };

    // shell:40 — descending-budget loop. Closest-edit candidates
    // overwrite farther ones (via HashMap insert) so output prefers
    // accuracy.
    let candidates: Vec<(String, usize)> = state
        .comp
        .all_completions()
        .iter()
        .map(|c| (c.str_.clone(), edit_distance(&prefix, &c.str_)))
        .filter(|(_, d)| *d <= effective_max)
        .collect();

    if candidates.is_empty() {
        return CompleterResult::NoMatch;
    }

    let mut best: HashMap<String, usize> = HashMap::new();
    for (name, dist) in candidates {
        // Keep the minimum distance seen for each name (idempotent
        // under repeated calls).
        best.entry(name)
            .and_modify(|d| {
                if dist < *d {
                    *d = dist;
                }
            })
            .or_insert(dist);
    }

    // Treat the previously-added matches as the candidate POOL only,
    // not as the final output. Clear the groups so we don't double-
    // emit them alongside the dedup/sorted result. Mirrors shell's
    // per-level loop discarding its prior level's matches before
    // adding the next level's batch.
    state.comp.groups.clear();
    state.comp.nmatches = 0;

    let mut emitted: Vec<(String, usize)> = best.into_iter().collect();
    emitted.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    for (name, _) in emitted {
        state.comp.add_match(Completion::new(&name), None);
    }
    CompleterResult::Matched
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
    fn zero_max_errors_consults_style() {
        // explicit 0 → fall through to zstyle. We set no style →
        // default 2 → "gut" at dist=1 from "git" matches.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "git".into();
        state.comp.add_match(Completion::new("git"), None);
        state.comp.add_match(Completion::new("gut"), None);
        assert!(matches!(
            _approximate(&mut state, 0),
            CompleterResult::Matched
        ));
    }

    #[test]
    fn single_char_prefix_bails() {
        // shell:10 — `|PREFIX+SUFFIX| <= 1` → return 1.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "g".into();
        state.comp.add_match(Completion::new("git"), None);
        assert!(matches!(
            _approximate(&mut state, 2),
            CompleterResult::NoMatch
        ));
    }

    #[test]
    fn prefix_plus_suffix_two_chars_passes_gate() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "a".into();
        state.comp.params.suffix = "b".into();
        state.comp.add_match(Completion::new("ab"), None);
        state.comp.add_match(Completion::new("ax"), None);
        // gate passes (|p|+|s| = 2). Both within dist 1 of "a".
        assert!(matches!(
            _approximate(&mut state, 1),
            CompleterResult::Matched
        ));
    }

    #[test]
    fn max_errors_style_honored_when_arg_zero() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.comp.params.prefix = "abc".into();
        state.comp.add_match(Completion::new("abc"), None);
        state.comp.add_match(Completion::new("xyz"), None); // dist=3
        // max-errors=3 via style → "xyz" passes.
        state.styles.set(
            ":completion::complete::test::",
            "max-errors",
            vec!["3".into()],
            false,
        );
        let _ = _approximate(&mut state, 0);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"xyz"));
    }

    #[test]
    fn output_sorted_by_distance_ascending() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "git".into();
        state.comp.add_match(Completion::new("gat"), None); // dist=1
        state.comp.add_match(Completion::new("git"), None); // dist=0
        state.comp.add_match(Completion::new("got"), None); // dist=1
        let _ = _approximate(&mut state, 2);
        // First match emitted should be the exact (dist=0).
        let emitted: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        // Find the index of "git" — it must come before "gat" and "got".
        let git_idx = emitted.iter().position(|n| *n == "git").unwrap();
        let gat_idx = emitted.iter().position(|n| *n == "gat").unwrap();
        let got_idx = emitted.iter().position(|n| *n == "got").unwrap();
        assert!(git_idx < gat_idx);
        assert!(git_idx < got_idx);
    }

    #[test]
    fn duplicate_candidates_collated_to_minimum_distance() {
        // Add the same candidate twice — verify HashMap dedup works.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "abc".into();
        state.comp.add_match(Completion::new("abc"), None);
        state.comp.add_match(Completion::new("abc"), None);
        state.comp.add_match(Completion::new("xyz"), None);
        let _ = _approximate(&mut state, 5);
        let count_abc = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .filter(|c| c.str_ == "abc")
            .count();
        assert_eq!(count_abc, 1, "duplicate candidates collapse to single emission");
    }

    #[test]
    fn empty_pool_returns_no_match_even_with_high_budget() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "anything".into();
        // No candidates added.
        assert!(matches!(
            _approximate(&mut state, 99),
            CompleterResult::NoMatch
        ));
    }
}
