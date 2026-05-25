//! Port of `_widgets` from `Completion/Zsh/Type/_widgets`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl pattern
//! sh: 4
//! sh: 5  pattern=( -g \* )
//! sh: 6  zparseopts -D -K -E g:=pattern
//! sh: 7
//! sh: 8  _description widgets expl widget
//! sh: 9  compadd "$@" "$expl[@]" -M 'r:|-=* r:|=*' - "${(@k)widgets[(R)${pattern[2]}]}"
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
