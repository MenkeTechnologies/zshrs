//! Port of `_expand_word` from `Completion/Base/Widget/_expand_word`.
//!
//! Full upstream body (13 lines verbatim):
//! ```text
//! sh: 1  #compdef -K _expand_word complete-word \C-xe _list_expansions list-choices \C-xd
//! sh: 2
//! sh: 3  # Simple completion front-end implementing expansion.
//! sh: 4
//! sh: 5  local curcontext="$curcontext"
//! sh: 6
//! sh: 7  if [[ -z "$curcontext" ]]; then
//! sh: 8    curcontext="expand-word:::"
//! sh: 9  else
//! sh:10    curcontext="expand-word:${curcontext#*:}"
//! sh:11  fi
//! sh:12
//! sh:13  _main_complete _expand
//! ```
//!
//! The shell version sets curcontext to `expand-word:…` and runs
//! `_main_complete _expand`. Our Rust port shortcuts: directly call
//! `_expand` (the expansion completer). User-visible behavior
//! identical because `_main_complete` would just dispatch to
//! `_expand` anyway.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
