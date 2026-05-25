//! Port of `_assign` from `Completion/Zsh/Context/_assign`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh: 1  #compdef -assign-parameter-
//! sh: 2
//! sh: 3  _parameters -g "^*readonly*" -S ''
//! ```
//!
//! The shell function has no `local` declarations and no positional
//! parameters — it dispatches a single `_parameters` call with
//! `-g <pattern>` (extended-glob exclusion of read-only params) and
//! `-S ''` (empty auto-suffix, suppressing the default space).
//!
//! Signature divergence (`// rust:`): shell `_assign` takes no args
//! and reads `$words` / `$compstate` globals; Rust port threads
//! `state` + `params` (the hash of `$parameters`) explicitly because
//! compsys-Rust has no process-global equivalent of zsh's
//! `(P)$parameters`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
