//! Port of `_dynamic_directory_name` from `Completion/Zsh/Context/_dynamic_directory_name`.
//!
//! Full upstream body (29 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  local -a dirfuncs=(
//! sh: 3      ${(k)functions[zsh_directory_name]}
//! sh: 4      $zsh_directory_name_functions
//! sh: 5  )
//! sh: 6  local descr='dynamically named directory'
//! sh: 7
//! sh: 8  if (( $#dirfuncs )); then
//! sh: 9    local -a expl
//! sh:10    local -i ret
//! sh:11    local func suf tag=dynamically-named-directories
//! sh:12
//! sh:13    [[ $ISUFFIX != \]* ]] &&
//! sh:14        suf=-S]
//! sh:15
//! sh:16    _tags "$tag"
//! sh:17    while _tags; do
//! sh:18      while _next_label "$tag" expl "$descr" $suf; do
//! sh:19        for func in $dirfuncs; do
//! sh:20          $func c && ret=0
//! sh:21        done
//! sh:22      done
//! sh:23      (( ret )) || break
//! sh:24    done
//! sh:25    return ret
//! sh:26
//! sh:27  else
//! sh:28    _message "${descr}: implement as zsh_directory_name c"
//! sh:29  fi
//! ```
//!
//! Strict Rust port: dispatches each registered
//! `zsh_directory_name_functions` callback via [`_call_function`]
//! with `"c"` as the conceptual arg. Falls back to `_message` when
//! no callbacks are registered.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
