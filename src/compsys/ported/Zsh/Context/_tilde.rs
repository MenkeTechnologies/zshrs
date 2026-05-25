//! Port of `_tilde` from `Completion/Zsh/Context/_tilde`.
//!
//! Full upstream body (32 lines verbatim):
//! ```text
//! sh: 1  #compdef -tilde-
//! sh: 2
//! sh: 3  # We use all named directories and user names here. If this is too slow
//! sh: 4  # for you or if there are too many of them, you may want to use
//! sh: 5  # `compadd -qS/ -a friends' or something like that.
//! sh: 6
//! sh: 7  [[ -n "$compstate[quote]" ]] && return 1
//! sh: 8
//! sh: 9  local expl suf ret=1
//! sh:10
//! sh:11  if [[ "$SUFFIX" = */* ]]; then
//! sh:12    ISUFFIX="/${SUFFIX#*/}$ISUFFIX"
//! sh:13    SUFFIX="${SUFFIX%%/*}"
//! sh:14    suf=(-S '')
//! sh:15  else
//! sh:16    suf=(-qS/)
//! sh:17  fi
//! sh:18
//! sh:19  _tags users named-directories directory-stack
//! sh:20
//! sh:21  while _tags; do
//! sh:22    _requested users && _users "$suf[@]" "$@" && ret=0
//! sh:23
//! sh:24    _requested named-directories expl 'named directory' \
//! sh:25        compadd "$suf[@]" "$@" -k nameddirs && ret=0
//! sh:26
//! sh:27    _requested directory-stack && _directory_stack "$suf[@]" && ret=0
//! sh:28
//! sh:29    (( ret )) || return 0
//! sh:30  done
//! sh:31
//! sh:32  return ret
//! ```
//!
//! Strict Rust port: faithful 1:1 — dispatches via `_tags` + the
//! three `_requested` per-tag branches. Caller supplies user
//! homedirs, named dirs, and dirstack.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
