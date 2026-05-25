//! Port of `_sub_commands` from `Completion/Base/Utility/_sub_commands`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  if [[ CURRENT -eq 2 ]]; then
//! sh: 6    _wanted commands expl command compadd "$@"
//! sh: 7  else
//! sh: 8    _message 'no more arguments'
//! sh: 9  fi
//! ```
//!
//! Upstream emits the supplied commands when at position 2 (right
//! after the main command name), or "no more arguments" otherwise.
//!
//! Strict Rust port: honors the `current==2` position gate
//! (shell:5). When at position 2, emits the supplied commands;
//! otherwise dispatches `_message 'no more arguments'`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
