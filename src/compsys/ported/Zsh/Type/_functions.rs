//! Port of `_functions` from `Completion/Zsh/Type/_functions`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #compdef unfunction
//! sh: 2
//! sh: 3  local expl ffilt
//! sh: 4
//! sh: 5  zstyle -t ":completion:${curcontext}:functions" prefix-needed && \
//! sh: 6   [[ $PREFIX != [_.]* ]] && \
//! sh: 7   ffilt='[(I)[^_.]*]'
//! sh: 8
//! sh: 9  _wanted functions expl 'shell function' compadd -k "$@" - "functions$ffilt"
//! ```
//!
//! `prefix-needed` semantics: when truthy AND user hasn't typed
//! something starting with `_` or `.`, hide names that DO start
//! with `_` or `.` (i.e. only show "public" functions).
//!
//! Strict Rust port: caller injects the function name list (since
//! compsys can't see the parent's `$functions` assoc).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
