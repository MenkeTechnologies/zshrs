//! Port of `_cmdambivalent` from `Completion/Unix/Type/_cmdambivalent`.
//!
//! Full upstream body (17 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  if (( CURRENT == 1 && ${#words} == 1 )); then
//! sh: 4    # Heuristics to decide whether to complete for system() or for execl().
//! sh: 5    local space=' '
//! sh: 6    if (( ${${words[CURRENT]}[(I)$space]} )); then
//! sh: 7      _cmdstring
//! sh: 8    elif [[ ${${compstate[all_quotes]}[1]} == (\'|\") ]]; then
//! sh: 9      _cmdstring
//! sh:10    else
//! sh:11      _command_names -e
//! sh:12    fi
//! sh:13  elif (( CURRENT == 1 )); then
//! sh:14    _command_names -e
//! sh:15  else
//! sh:16    _normal
//! sh:17  fi
//! ```
//!
//! Strict Rust port: ports the full 5-way branch:
//! 1. `current==1 && #words==1` + word has space    → `_cmdstring`
//! 2. `current==1 && #words==1` + word is quoted   → `_cmdstring`
//! 3. `current==1 && #words==1` + bare             → `_command_names -e`
//! 4. `current==1` (other)                          → `_command_names -e`
//! 5. else (argument position)                      → `_normal`
//!
//! Caller supplies `WordKind` describing the user's first word so
//! the leaf layer doesn't have to inspect raw quote state.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
