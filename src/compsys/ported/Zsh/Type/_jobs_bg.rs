//! Port of `_jobs_bg` from `Completion/Zsh/Type/_jobs_bg`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh: 1  #compdef bg
//! sh: 2
//! sh: 3  _jobs -s "$@"
//! ```
//!
//! Strict Rust port: thin alias for `_jobs(..., JobsFilter::Suspended,
//! false)`. Suspended jobs are the only valid `bg` targets.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
