//! Port of `_nothing` from `Completion/Base/Utility/_nothing`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh: 1  #compdef true false log times clear logname whoami sync
//! sh: 2
//! sh: 3  _message 'no argument or option'
//! ```
//!
//! Upstream is bound as the completion for commands that genuinely
//! take no args (`true`, `false`, `whoami`, etc.) — it emits the
//! "no argument or option" message via `_message`.
//!
//! Strict Rust port: dispatches to `_message` directly. The shell
//! `_message` itself returns success regardless of whether matches
//! were added; we mirror by always returning true.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
