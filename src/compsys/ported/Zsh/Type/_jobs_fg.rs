//! Port of `_jobs_fg` from `Completion/Zsh/Type/_jobs_fg`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh: 1  #compdef disown fg
//! sh: 2
//! sh: 3  _jobs "$@"
//! ```
//!
//! Strict Rust port: alias for `_jobs(..., JobsFilter::All, false)`.
//! `fg` and `disown` accept any job (running OR suspended) — no
//! filter applied.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
