//! Port of `_generic` from `Completion/Base/Widget/_generic`.
//!
//! Full upstream body (18 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  if [[ -n $ZSH_TRACE_GENERIC_WIDGET ]]; then
//! sh: 4    local widget=$ZSH_TRACE_GENERIC_WIDGET
//! sh: 5    unset ZSH_TRACE_GENERIC_WIDGET
//! sh: 6    $widget _generic
//! sh: 7    return
//! sh: 8  fi
//! sh: 9
//! sh:10  local curcontext="${curcontext:-}"
//! sh:11
//! sh:12  if [[ -z "$curcontext" ]]; then
//! sh:13    curcontext="${WIDGET}:::"
//! sh:14  else
//! sh:15    curcontext="${WIDGET}:${curcontext#*:}"
//! sh:16  fi
//! sh:17
//! sh:18  _main_complete "$@"
//! ```
//!
//! Strict Rust port: implements both branches.
//!
//! 1. If `trace_widget` is non-empty, dispatch the trace widget via
//! `_call_function` (mirrors `$ZSH_TRACE_GENERIC_WIDGET`).
//! 2. Otherwise rewrite `curcontext` to `widget:rest-of-context`
//! (shell:12-16) and then invoke `action` (the caller's
//! `_main_complete` equivalent — the closure form lets the
//! caller wire whichever completer chain they want).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
