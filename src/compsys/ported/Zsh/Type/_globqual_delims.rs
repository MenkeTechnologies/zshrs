//! Port of `_globqual_delims` from `Completion/Zsh/Type/_globqual_delims`.
//!
//! Full upstream body (24 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Helper for _globquals.  Sets delim to delimiter to match.
//! sh: 4
//! sh: 5  # don't restore special parameters
//! sh: 6  compstate[restore]=no
//! sh: 7
//! sh: 8  delim=$PREFIX[1]
//! sh: 9  compset -p 1
//! sh:10
//! sh:11  # One of matching brackets?
//! sh:12  # These don't actually work: the parser gets very confused.
//! sh:13  local matchl="<({[" matchr=">)}]"
//! sh:14  integer ind=${matchl[(I)$delim]}
//! sh:15
//! sh:16  (( ind )) && delim=$matchr[ind]
//! sh:17
//! sh:18  if compset -P "[^$delim]#$delim"; then
//! sh:19    # Completely matched.
//! sh:20    return 0
//! sh:21  else
//! sh:22    # Still in delimiter
//! sh:23    return 1
//! sh:24  fi
//! ```
//!
//! The fn picks the OPEN delim char (`<`, `(`, `{`, `[` map to their
//! matching close; anything else is the close itself), then checks
//! whether `PREFIX` already contains a closing delim somewhere.
//!
//! Strict Rust port: returns the (open, close, already_closed) tuple
//! based on inspecting state.params.prefix. Mutates `iprefix`/`prefix`
//! to mirror the shell's `compset -p 1` (chew the open delim).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
