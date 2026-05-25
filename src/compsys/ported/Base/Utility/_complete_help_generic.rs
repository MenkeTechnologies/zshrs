//! Port of `_complete_help_generic` from `Completion/Base/Utility/_complete_help_generic`.
//!
//! Full upstream body (17 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Note this is a normal ZLE widget, not a completion widget.
//! sh: 4  # A completion widget can't call another widget, while a normal
//! sh: 5  # widget can.
//! sh: 6
//! sh: 7  [[ $WIDGET = *noread* ]] || local ZSH_TRACE_GENERIC_WIDGET
//! sh: 8
//! sh: 9  if [[ $WIDGET = *debug* ]]; then
//! sh:10    ZSH_TRACE_GENERIC_WIDGET=_complete_debug
//! sh:11  else
//! sh:12    ZSH_TRACE_GENERIC_WIDGET=_complete_help
//! sh:13  fi
//! sh:14
//! sh:15  if [[ $WIDGET != *noread* ]]; then
//! sh:16    zle read-command && zle $REPLY -w
//! sh:17  fi
//! ```
//!
//! Upstream is a `zle` widget that reads ANOTHER widget name from
//! the user via `read-command`, then runs it with
//! `ZSH_TRACE_GENERIC_WIDGET` set so `_generic` knows to call
//! `_complete_help` (or `_complete_debug`) on it.
//!
//! Simplified Rust port: takes the `--help`-style text directly and
//! parses dash-prefixed option lines, emitting them as completions
//! with `option -- description` disp format. Skips the zle widget
//! interaction entirely — this is the "give me the parsed options"
//! API that callers actually need.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
