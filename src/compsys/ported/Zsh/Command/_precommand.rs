//! Port of `_precommand` from `Completion/Zsh/Command/_precommand`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh: 1  #compdef - nohup eval time rusage noglob nocorrect catchsegv aoss hilite eatmydata
//! sh: 2
//! sh: 3  shift words
//! sh: 4  (( CURRENT-- ))
//! sh: 5
//! sh: 6  _normal -p $service
//! ```
//!
//! Upstream pops the precommand off `$words`, decrements CURRENT
//! (since we're now one word back), and runs `_normal -p $service`
//! (`-p` tells _normal "this is precommand dispatch").
//!
//! Strict Rust port: mirrors `shift words; (( CURRENT-- )); _normal`.
//! Pops the leading word from `state.comp.params.words` and
//! decrements `current` before calling `_normal`. Restores both
//! after — `_precommand` is a one-shot dispatch helper, not a
//! mutation primitive.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
