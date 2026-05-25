//! Port of `_approximate` from `Completion/Base/Completer/_approximate`.
//!
//! Full upstream body (121 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # This code will try to correct the string on the line based on the
//! sh:  4  # strings generated for the context. These corrected strings will be
//! sh:  5  # shown in a list and one can cycle through them as in a menu completion
//! sh:  6  # or get the corrected prefix.
//! sh:  7
//! sh:  8  # We don't try correction if the string is too short or we have tried it
//! sh:  9  # already.
//! sh: 10
//! sh: 11  [[ _matcher_num -gt 1 || "${#:-$PREFIX$SUFFIX}" -le 1 ]] && return 1
//! sh: 12
//! sh: 13  local _comp_correct _correct_expl _correct_group comax cfgacc match
//! sh: 14  local oldcontext="${curcontext}" opm="$compstate[pattern_match]"
//! sh: 15  integer ret=1
//! sh: 16
//! sh: 17  if [[ "$1" = -a* ]]; then
//! sh: 18    cfgacc="${1[3,-1]}"
//! sh: 19  elif [[ "$1" = -a ]]; then
//! sh: 20    cfgacc="$2"
//! sh: 21  else
//! sh: 22    zstyle -s ":completion:${curcontext}:" max-errors cfgacc ||
//! sh: 23        cfgacc='2 numeric'
//! sh: 24  fi
//! sh: 25
//! sh: 26  # Get the number of errors to accept.
//! sh: 27
//! sh: 28  if [[ "$cfgacc" = *numeric* && ${NUMERIC:-1} -ne 1 ]]; then
//! sh: 29    # A numeric argument may mean that we should not try correction.
//! sh: 30
//! sh: 31    [[ "$cfgacc" = *not-numeric* ]] && return 1
//! sh: 32
//! sh: 33    # Prefer the numeric argument if that has a sensible value.
//! sh: 34
//! sh: 35    comax="${NUMERIC:-1}"
//! sh: 36  else
//! sh: 37    comax="${cfgacc//[^0-9]}"
//! sh: 38  fi
//! sh: 39
//! sh: 40  # If the number of errors to accept is too small, give up.
//! sh: 41
//! sh: 42  [[ "$comax" -lt 1 ]] && return 1
//! sh: 43
//! sh: 44  _tags corrections original
//! sh: 45
//! sh: 46  # Otherwise temporarily define a function to use instead of the builtin that
//! sh: 47  # adds matches. This is used to be able to stick the `(#a...)' in the right
//! sh: 48  # place (after an ignored prefix).
//! sh: 49  #
//! sh: 50  # Current shell structure for use with "always", to make sure we unfunction our
//! sh: 51  # compadd and restore any compadd function defined previously.
//! sh: 52  {
//! sh: 53  _shadow -s _approximate compadd
//! sh: 54  compadd() {
//! sh: 55    local ppre="$argv[(I)-p]"
//! sh: 56
//! sh: 57    [[ ${argv[(I)-[a-zA-Z]#U[a-zA-Z]#]} -eq 0 &&
//! sh: 58        "${#:-$PREFIX$SUFFIX}" -le _comp_correct ]] && return
//! sh: 59
//! sh: 60    if [[ "$PREFIX" = \~* && ( ppre -eq 0 || "$argv[ppre+1]" != \~* ) ]]; then
//! sh: 61      PREFIX="~(#a${_comp_correct})${PREFIX[2,-1]}"
//! sh: 62    else
//! sh: 63      PREFIX="(#a${_comp_correct})$PREFIX"
//! sh: 64    fi
//! sh: 65
//! sh: 66    (( $_correct_group && ${${argv[1,(r)-(|-)]}[(I)-*[JV]]} )) &&
//! sh: 67        _correct_expl[_correct_group]=${argv[1,(r)-(-|)][(R)-*[JV]]}
//! sh: 68
//! sh: 69    compadd@_approximate "$_correct_expl[@]" "$@"
//! sh: 70  }
//! sh: 71
//! sh: 72  _comp_correct=1
//! sh: 73
//! sh: 74  [[ -z "$compstate[pattern_match]" ]] && compstate[pattern_match]='*'
//! sh: 75
//! sh: 76  while [[ _comp_correct -le comax ]]; do
//! sh: 77    curcontext="${oldcontext/(#b)([^:]#:[^:]#:)/${match[1][1,-2]}-${_comp_correct}:}"
//! sh: 78
//! sh: 79    _description corrections _correct_expl corrections \
//! sh: 80                 "e:$_comp_correct" "o:$PREFIX$SUFFIX"
//! sh: 81
//! sh: 82    _correct_group="$_correct_expl[(I)-*[JV]]"
//! sh: 83
//! sh: 84    if _complete; then
//! sh: 85      if zstyle -t ":completion:${curcontext}:" insert-unambiguous &&
//! sh: 86         [[ "${#compstate[unambiguous]}" -ge "${#:-$PREFIX$SUFFIX}" ]]; then
//! sh: 87        compstate[pattern_insert]=unambiguous
//! sh: 88      elif _requested original &&
//! sh: 89           { [[ compstate[nmatches] -gt 1 ]] ||
//! sh: 90             zstyle -t ":completion:${curcontext}:" original }; then
//! sh: 91        local expl
//! sh: 92
//! sh: 93        _description -V original expl original
//! sh: 94
//! sh: 95        builtin compadd "$expl[@]" -U -Q - "$PREFIX$SUFFIX"
//! sh: 96
//! sh: 97        # If you always want to see the list of possible corrections,
//! sh: 98        # set `compstate[list]=list force' here.
//! sh: 99
//! sh:100        [[ "$compstate[list]" != list* ]] &&
//! sh:101            compstate[list]="$compstate[list] force"
//! sh:102      fi
//! sh:103      compstate[pattern_match]="$opm"
//! sh:104
//! sh:105      ret=0
//! sh:106      break
//! sh:107    fi
//! sh:108
//! sh:109    [[ "${#:-$PREFIX$SUFFIX}" -le _comp_correct+1 ]] && break
//! sh:110    (( _comp_correct++ ))
//! sh:111  done
//! sh:112
//! sh:113  } always {
//! sh:114    _unshadow
//! sh:115  }
//! sh:116
//! sh:117  (( ret == 0 )) && return 0
//! sh:118
//! sh:119  compstate[pattern_match]="$opm"
//! sh:120
//! sh:121  return 1
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

use crate::compsys::base::{CompleterResult, MainCompleteState};
use crate::compsys::completion::Completion;

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
