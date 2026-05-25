//! Port of `_as_if` from `Completion/Base/Utility/_as_if`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  local words=("$words[@]") CURRENT=$CURRENT
//! sh: 3  local _comp_command1 _comp_command2 _comp_command
//! sh: 4
//! sh: 5  words[1]=("$@")
//! sh: 6  (( CURRENT += $# - 1 ))
//! sh: 7
//! sh: 8  _set_command
//! sh: 9
//! sh:10  _dispatch "$_comp_command" "$_comp_command1" "$_comp_command2" -default-
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
