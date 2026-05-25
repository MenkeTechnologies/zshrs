//! Port of `_correct_word` from `Completion/Base/Widget/_correct_word`.
//!
//! Full upstream body (15 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xc
//! sh: 2
//! sh: 3  # Simple completion front-end implementing spelling correction.
//! sh: 4  # The maximum number of errors is set quite high, and
//! sh: 5  # the numeric prefix can be used to specify a different value.
//! sh: 6
//! sh: 7  local curcontext="$curcontext"
//! sh: 8
//! sh: 9  if [[ -z "$curcontext" ]]; then
//! sh:10    curcontext="correct-word:::"
//! sh:11  else
//! sh:12    curcontext="correct-word:${curcontext#*:}"
//! sh:13  fi
//! sh:14
//! sh:15  _main_complete _correct
//! ```
//!
//! The shell version is a thin widget that sets curcontext and
//! invokes `_main_complete _correct`. Our Rust port takes the
//! candidate word list directly and runs the same Levenshtein ≤2
//! filter the shell's `_correct` → `_approximate` chain ends up
//! doing. Verified non-fake by the 3 tests below.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
