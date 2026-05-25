//! Port of `_options_set` from `Completion/Zsh/Type/_options_set`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete all set options. This relies on `_main_complete' to store the
//! sh: 4  # names of the options that were set when it was called in the array
//! sh: 5  # `_options_set'.
//! sh: 6
//! sh: 7  local expl
//! sh: 8
//! sh: 9  _wanted zsh-options expl 'set zsh option' \
//! sh:10      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -a - _options_set
//! ```
//!
//! Upstream uses `${(@k)options[(R)on]}` to get the keys of the
//! shell `$options` array whose VALUE is "on" (i.e., set).
//!
//! Faithful Rust port: filters the caller's `&[(name, is_set)]`
//! slice to set-only entries and delegates to `_options` for the
//! actual emit (so disp formatting stays consistent).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
