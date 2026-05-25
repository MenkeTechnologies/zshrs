//! Port of `_wanted` from `Completion/Base/Core/_wanted`.
//!
//! Full upstream body (13 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local -a __targs __gopt
//! sh: 4
//! sh: 5  zparseopts -D -a __gopt 1 2 V J x C:=__targs
//! sh: 6
//! sh: 7  _tags "$__targs[@]" "$1"
//! sh: 8
//! sh: 9  while _tags; do
//! sh:10    _all_labels "$__gopt[@]" "$@" && return 0
//! sh:11  done
//! sh:12
//! sh:13  return 1
//! ```
//!
//! Simplified Rust port: skips the `-C context` / -1 / -2 / -V / -J
//! / -x compadd flag-parsing (`__gopt` accumulator) because the Rust
//! port takes an action closure instead of a compadd command line.
//! The user-visible contract — "if tag is requested, begin its group
//! + run action + close group + return action's result" — IS
//! preserved.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
