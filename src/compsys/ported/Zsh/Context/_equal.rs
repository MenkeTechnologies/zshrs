//! Port of `_equal` from `Completion/Zsh/Context/_equal`.
//!
//! Full upstream body (11 lines verbatim):
//! ```text
//! sh: 1  #compdef -equal-
//! sh: 2
//! sh: 3  local -a match mbegin mend
//! sh: 4
//! sh: 5  if _have_glob_qual $PREFIX; then
//! sh: 6    compset -p ${#match[1]}
//! sh: 7    compset -S '[^\)\|\~]#(|\))'
//! sh: 8    _globquals
//! sh: 9  else
//! sh:10    _path_commands
//! sh:11  fi
//! ```
//!
//! Strict Rust port: faithful 1:1 — delegates to our ported
//! [`_path_commands`].

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
