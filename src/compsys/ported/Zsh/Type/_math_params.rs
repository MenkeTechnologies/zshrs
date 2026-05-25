//! Port of `_math_params` from `Completion/Zsh/Type/_math_params`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  _parameters -g '(integer|float)*' || _parameters
//! ```
//!
//! Strict Rust port: faithful 1:1 — calls our ported
//! [`_parameters_with_opts`] with the exact pattern shell uses;
//! on `false` falls back to [`_parameters`] no-filter, matching
//! the shell `||` operator.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
