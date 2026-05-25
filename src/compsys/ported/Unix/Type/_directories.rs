//! Port of `_directories` from `Completion/Unix/Type/_directories`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh: 1  #compdef dircmp -P -value-,*path,-default-
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  _wanted directories expl directory _files -/ "$@" -
//! ```
//!
//! Faithful re-port: mirrors shell-side `_wanted <tag> <expl_arr> <descr>
//! <cmd>` invocation by calling our ported [`_wanted`] helper with the
//! same `tag = "directories"` and `descr = "directory"`. Inner `_files -/`
//! is `directories_execute` (our `_files.rs` port specialised for the
//! `-/` flag = directories-only).
//!
//! The shell local `expl` is the description-array name passed to the
//! `_wanted` machinery; in the Rust port the description threads
//! through `_wanted`'s third argument, so `expl` is internal to the
//! helper and not a Rust-side local. Documented as a `// sh:3` marker
//! for traceability.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
