//! Port of `_redirect` from `Completion/Zsh/Context/_redirect`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #compdef -redirect-
//! sh: 2
//! sh: 3  local strs _comp_command1 _comp_command2 _comp_command
//! sh: 4
//! sh: 5  _set_command
//! sh: 6
//! sh: 7  strs=( -default- )
//! sh: 8
//! sh: 9  if [[ "$CURRENT" != "1" ]]; then
//! sh:10    strs=( "${_comp_command}" "$strs[@]" )
//! sh:11    if [[ -n "$_comp_command1" ]]; then
//! sh:12      strs=( "${_comp_command1}" "$strs[@]" )
//! sh:13      [[ -n "$_comp_command2" ]] &&
//! sh:14        strs=( "${_comp_command2}" "$strs[@]" )
//! sh:15    fi
//! sh:16  fi
//! sh:17
//! sh:18  _dispatch -redirect-,${compstate[redirect]},$_comp_command \
//! sh:19  	  -redirect-,{${compstate[redirect]},-default-},${^strs}
//! ```
//!
//! Strict Rust port: faithful 1:1 — calls our ported
//! [`_set_command`] to populate `_comp_command{,1,2}` in
//! `state.lastcomp`, builds the dispatch-key list per upstream,
//! then calls our ported [`_dispatch`].

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
