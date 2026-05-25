//! Port of `_parameter` from `Completion/Zsh/Context/_parameter`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh: 1  #compdef -parameter-
//! sh: 2
//! sh: 3  if compset -P '*:'; then
//! sh: 4    _history_modifiers p
//! sh: 5    return
//! sh: 6  fi
//! sh: 7
//! sh: 8  _parameters -e
//! ```
//!
//! `_parameters` upstream parses ONLY `-g` via `zparseopts`; every
//! other arg is forwarded to `compadd "$@"` as a passthrough. The
//! `-e` flag has no documented compadd meaning in current zsh and
//! is silently absorbed there; the user-visible behavior of
//! `_parameter` is therefore identical to bare `_parameters`.
//!
//! Strict Rust port: faithful 1:1 — single-line delegate to
//! [`_parameters`].

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
