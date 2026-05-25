//! Port of `_cmdstring` from `Completion/Unix/Type/_cmdstring`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This is for a quoted argument that will be interpreted as a command.
//! sh: 4
//! sh: 5  compset -q
//! sh: 6  _normal
//! ```
//!
//! Upstream calls `compset -q` to treat the current word as a
//! quoted shell argument, then dispatches to `_normal` (which
//! handles command-vs-argument selection).
//!
//! Strict Rust port: applies `compset -q` semantics (strip
//! surrounding single OR double quotes from `prefix`/`suffix`)
//! BEFORE dispatching to `_command_names`. The shell `compset -q`
//! re-tokenizes the current word as a shell argument; for the
//! common "user typed `eval 'pre|fix'`" case, stripping the outer
//! quotes is the dominant effect.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
