//! Port of `_guard` from `Completion/Base/Utility/_guard`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local garbage
//! sh: 4
//! sh: 5  zparseopts -K -D -a garbage M+: J+: V+: 1 2 o+: n F: X+:
//! sh: 6
//! sh: 7  [[ "$PREFIX$SUFFIX" != $~1 ]] && return 1
//! sh: 8
//! sh: 9  shift
//! sh:10  _message -e "$*"
//! sh:11
//! sh:12  [[ -n "$PREFIX$SUFFIX" ]]
//! ```
//!
//! Upstream uses the `[[ X = $~1 ]]` pattern-match (where `$~1`
//! enables glob-expansion). If the user-typed word doesn't match
//! the guard pattern, return 1 to skip this branch. Otherwise emit
//! a descriptive `_message -e` and return based on whether anything
//! is typed.
//!
//! Simplified Rust port: returns true iff PREFIX matches the
//! supplied glob (`*`/`?`) OR (for literal patterns) starts with it.
//! Drops the `_message -e` emission + compadd opt consumption — the
//! Rust caller's downstream completer handles the user feedback.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
