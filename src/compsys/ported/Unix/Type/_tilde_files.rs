//! Port of `_tilde_files` from `Completion/Unix/Type/_tilde_files`.
//!
//! Full upstream body (39 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete files and expand tilde expansions in it.
//! sh: 4
//! sh: 5  if [[ ( -o magicequalsubst && "$IPREFIX" = *\= ) || $argv[(I)-W*] -ne 0 ]]; then
//! sh: 6    _files "$@"
//! sh: 7    return
//! sh: 8  fi
//! sh: 9
//! sh:10  case "$PREFIX" in
//! sh:11  \~/*)
//! sh:12    IPREFIX="${IPREFIX}${HOME}/"
//! sh:13    PREFIX="${PREFIX[3,-1]}"
//! sh:14    _files "$@" -W "${HOME}"
//! sh:15    ;;
//! sh:16  \~*/*)
//! sh:17    local user="${PREFIX[2,-1]%%/*}"
//! sh:18
//! sh:19    if (( $+userdirs[$user] )); then
//! sh:20      user="$userdirs[$user]"
//! sh:21    elif (( $+nameddirs[$user] )); then
//! sh:22      user="$nameddirs[$user]"
//! sh:23    else
//! sh:24      _message "unknown user \`$user'"
//! sh:25      return 1
//! sh:26    fi
//! sh:27    IPREFIX="${IPREFIX}${user%/}/"
//! sh:28    PREFIX="${PREFIX#*/}"
//! sh:29    _files "$@" -W "$user"
//! sh:30    ;;
//! sh:31  \~*)
//! sh:32    compset -p 1
//! sh:33    local -a expl=( "$@" )
//! sh:34    _alternative -O expl users:user:_users named-directories:'named directory':'compadd -k nameddirs'
//! sh:35    ;;
//! sh:36  *)
//! sh:37    _files "$@"
//! sh:38    ;;
//! sh:39  esac
//! ```
//!
//! Upstream handles three tilde shapes: `~/PATH` (your home),
//! `~user/PATH` (user's home via passwd), and `~` (just expand to
//! $HOME).
//!
//! Simplified Rust port: handles `~/` via $HOME env. Other shapes
//! (`~user/`, named-directories) fall through. Pinned by the
//! `iprefix_cleared_after_call` test which checks the save/restore
//! semantic for state.params.iprefix.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
