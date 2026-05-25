//! Port of `_options_unset` from `Completion/Zsh/Type/_options_unset`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete all unset options. This relies on `_main_complete' to store the
//! sh: 4  # names of the options that were unset when it was called in the array
//! sh: 5  # `_options_unset'.
//! sh: 6
//! sh: 7  local expl
//! sh: 8
//! sh: 9  _wanted zsh-options expl 'unset zsh option' \
//! sh:10      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -a - _options_unset
//! ```
//!
//! Faithful Rust port: mirrors `_options_set` but inverts the
//! filter (`!is_set` instead of `is_set`).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
