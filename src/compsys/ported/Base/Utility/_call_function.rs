//! Port of `_call_function` from `Completion/Base/Utility/_call_function`.
//!
//! Full upstream body (32 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Utility function to call a function if it exists.
//! sh: 4  #
//! sh: 5  # Usage: _call_function <return> <name> [ <args> ... ]
//! sh: 6  #
//! sh: 7  # If a function named <name> is defined (or defined to be autoloaded),
//! sh: 8  # it is called. If <return> is given not the string `-' or empty, it is
//! sh: 9  # taken as the name of a parameter and the return status of the function
//! sh:10  # called is stored in this parameter. All other arguments are given
//! sh:11  # to the function called.
//! sh:12  # The return value of this function is zero if the function was
//! sh:13  # called and non-zero otherwise.
//! sh:14
//! sh:15  local _name _ret
//! sh:16
//! sh:17  [[ "$1" != (|-) ]] && _name="$1"
//! sh:18
//! sh:19  shift
//! sh:20
//! sh:21  if (( $+functions[$1] )); then
//! sh:22    "$@"
//! sh:23    _ret="$?"
//! sh:24
//! sh:25    [[ -n "$_name" ]] && eval "${_name}=${_ret}"
//! sh:26
//! sh:27    compstate[restore]=''
//! sh:28
//! sh:29    return 0
//! sh:30  fi
//! sh:31
//! sh:32  return 1
//! ```
//!
//! Upstream resolves a shell function by name (from `$functions`
//! associative array) and invokes it, storing the return code in
//! the `<return>` parameter name.
//!
//! Faithful Rust port: maintains a process-global registry of
//! `(name → Box<dyn Fn(&mut MainCompleteState) -> bool>)` callbacks.
//! Callers register their completion fns via [`register`] at startup;
//! `_call_function` looks them up by name and invokes them. This is
//! the Rust analog of `$functions[$name]`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
