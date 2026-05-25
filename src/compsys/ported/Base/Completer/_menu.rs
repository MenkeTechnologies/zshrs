//! Port of `_menu` from `Completion/Base/Completer/_menu`.
//!
//! Full upstream body (23 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 4
//! sh: 5  # This completer is an example showing how menu completion can be
//! sh: 6  # implemented with the new completion system.
//! sh: 7  # Use this one before the normal _complete completer, as in:
//! sh: 8  #
//! sh: 9  #   zstyle ":completion:::::" completer _menu _complete
//! sh:10
//! sh:11  if [[ -n "$compstate[old_list]" ]]; then
//! sh:12
//! sh:13    # We have an old list, keep it and insert the next match.
//! sh:14
//! sh:15    compstate[old_list]=keep
//! sh:16    compstate[insert]=$((compstate[old_insert]+1))
//! sh:17  else
//! sh:18    # No old list, make completion insert the first match.
//! sh:19
//! sh:20    compstate[insert]=1
//! sh:21  fi
//! sh:22
//! sh:23  return 1
//! ```
//!
//! Strict Rust port: gates on `matcher_num > 1` (caller-supplied
//! since compsys's `CompletionState` doesn't expose it), then takes
//! the `old_list ? keep+bump : insert=1` branch and ALWAYS returns
//! false (shell `return 1`). The shell `return 1` is critical: it
//! signals to `_main_complete` that this completer DIDN'T provide
//! matches; the next completer (typically `_complete`) generates
//! them, and the inserted index drives which one fires.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
