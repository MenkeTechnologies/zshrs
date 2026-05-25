//! Port of `_completers` from `Completion/Zsh/Type/_completers`.
//!
//! STUB 2026-05-24 — original body was lost during cleanup. Re-port
//! using shell-side state + `bin_compadd`. Stub returns false.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
