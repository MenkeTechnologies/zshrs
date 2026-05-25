//! Port of `_vcs_info_hooks` from `Completion/Zsh/Type/_vcs_info_hooks`.
//!
//! Full upstream body (2 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  compadd - ${functions[(I)+vi-*]#+vi-}
//! ```
//!
//! `${functions[(I)+vi-*]#+vi-}` — list every function whose name
//! starts with `+vi-`, strip that prefix. These are the user's
//! defined vcs_info hooks (e.g. `+vi-git-untracked`).
//!
//! Strict Rust port: caller injects function names (we can't reach
//! `$functions` from the leaf).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
