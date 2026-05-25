//! Port of `_correct` from `Completion/Base/Completer/_correct`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This is mainly a wrapper around the more general `_approximate'.
//! sh: 4  # By setting `compstate[pattern_match]' to something unequal to `*' and
//! sh: 5  # then calling `_approximate', we get only corrections, not all strings
//! sh: 6  # with the corrected prefix and something after it.
//! sh: 7  #
//! sh: 8  # Supported configuration keys are the same as for `_approximate', only
//! sh: 9  # starting with `correct'.
//! sh:10
//! sh:11  local ret=1 opm="$compstate[pattern_match]"
//! sh:12
//! sh:13  compstate[pattern_match]='-'
//! sh:14
//! sh:15  _approximate && ret=0
//! sh:16
//! sh:17  compstate[pattern_match]="$opm"
//! sh:18
//! sh:19  return ret
//! ```
//!
//! Faithful Rust port: `_approximate(state, 1)` — pinning max_errors
//! to 1 mirrors shell's "only corrections" semantic (the compstate
//! [pattern_match]='-' trick prevents pattern-match acceptance,
//! leaving only Levenshtein-1 matches).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
