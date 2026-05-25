//! Port of `_arrays` from `Completion/Zsh/Type/_arrays`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh: 1  #compdef shift
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  _wanted arrays expl array _parameters "$@" -g '*array*'
//! ```
//!
//! Faithful re-port: mirrors shell-side `_wanted <tag> <expl_arr>
//! <descr> <cmd>` invocation by calling our ported [`_wanted`] with
//! `tag = "arrays"` and `descr = "array"`. Inner `_parameters -g
//! '*array*'` is [`_parameters_with_opts`] with `pattern: "*array*"`.
//!
//! The shell local `expl` is the description-array name threaded into
//! `_wanted`'s machinery; in the Rust port that role is implicit —
//! `_wanted`'s third arg carries the description. Documented at
//! `// sh:3` for traceability.
//!
//! Signature divergence (`// rust:`): shell `_arrays` reads `$parameters`
//! (a zsh special parameter mapping name → type) via the `_parameters`
//! callee. The Rust leaf can't reach `$parameters` directly, so the
//! caller passes a `HashMap<String, String>` snapshot.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
