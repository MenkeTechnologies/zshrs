//! Port of `_set_command` from `Completion/Base/Utility/_set_command`.
//!
//! Full upstream body (31 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This sets the parameters _comp_command1, _comp_command2 and _comp_command
//! sh: 4  # in the calling function.
//! sh: 5
//! sh: 6  local command
//! sh: 7
//! sh: 8  command="$words[1]"
//! sh: 9
//! sh:10  [[ -z "$command" ]] && return
//! sh:11
//! sh:12  if (( $+builtins[$command] + $+functions[$command] )); then
//! sh:13    _comp_command1="$command"
//! sh:14    _comp_command="$_comp_command1"
//! sh:15  elif [[ "$command[1]" = '=' ]]; then
//! sh:16    eval _comp_command2\=$command
//! sh:17    _comp_command1="$command[2,-1]"
//! sh:18    _comp_command="$_comp_command2"
//! sh:19  elif [[ "$command" = ..#/* ]]; then
//! sh:20    _comp_command1="${PWD}/$command"
//! sh:21    _comp_command2="${command:t}"
//! sh:22    _comp_command="$_comp_command2"
//! sh:23  elif [[ "$command" = */* ]]; then
//! sh:24    _comp_command1="$command"
//! sh:25    _comp_command2="${command:t}"
//! sh:26    _comp_command="$_comp_command2"
//! sh:27  else
//! sh:28    _comp_command1="$command"
//! sh:29    _comp_command2="$commands[$command]"
//! sh:30    _comp_command="$_comp_command1"
//! sh:31  fi
//! ```
//!
//! Upstream sets THREE parameters (_comp_command1/2, _comp_command)
//! with a 5-way branch on the shape of `$words[1]`:
//! 1. builtin / function    → 1=name,    2=unset, dispatch=1
//! 2. `=name`               → 1=name[2,-1] (stripped), 2=abs path
//! after `=` eval, dispatch=2
//! 3. `..#/...` (no /-only) → 1=$PWD/cmd, 2=basename, dispatch=2
//! 4. `*/*` (path)          → 1=cmd,      2=basename, dispatch=2
//! 5. else (PATH lookup)    → 1=cmd,      2=$commands[cmd],
//! dispatch=1
//!
//! Strict Rust port: stores all three under `lastcomp["_comp_command1"]`,
//! `lastcomp["_comp_command2"]`, `lastcomp["_comp_command"]` keys.
//! Builtin/function detection consults the inventory passed in by the
//! caller (compsys can't reach the parent crate's builtin/fn table).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
