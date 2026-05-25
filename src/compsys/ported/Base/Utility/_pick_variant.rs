//! Port of `_pick_variant` from `Completion/Base/Utility/_pick_variant`.
//!
//! Full upstream body (49 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local output cmd pat pre
//! sh: 4  local -a var
//! sh: 5  local -A opts
//! sh: 6
//! sh: 7  (( $+_cmd_variant )) || typeset -gA _cmd_variant
//! sh: 8
//! sh: 9  zparseopts -D -A opts b: c: r:
//! sh:10  : ${opts[-c]:=$words[1]}
//! sh:11
//! sh:12  while [[ $1 = *=* ]]; do
//! sh:13    var+=( "${1%%\=*}" "${1#*=}" )
//! sh:14    shift
//! sh:15  done
//! sh:16
//! sh:17  if (( ${#precommands:|builtin_precommands} )); then
//! sh:18    pre=command
//! sh:19  elif (( $+opts[-b] && ( $precommands[(I)builtin] || $+builtins[$opts[-c]] ) )); then
//! sh:20    (( $+opts[-r] )) && : ${(P)opts[-r]::=$opts[-b]}
//! sh:21    return 0
//! sh:22  elif (( $precommands[(I)builtin] )); then
//! sh:23    pre=builtin
//! sh:24  else
//! sh:25    # Neither builtin nor command-forcing precommand specified,
//! sh:26    # so no prefix is needed.
//! sh:27    pre=
//! sh:28  fi
//! sh:29
//! sh:30  if [[ $pre != builtin ]] && (( $+_cmd_variant[$opts[-c]] )); then
//! sh:31    (( $+opts[-r] )) && : ${(P)opts[-r]::=${_cmd_variant[$opts[-c]]}}
//! sh:32    [[ $_cmd_variant[$opts[-c]] = "$1" ]] && return 1
//! sh:33    return 0
//! sh:34  fi
//! sh:35
//! sh:36  output="$(_call_program variant $pre $opts[-c] "${@[2,-1]}" </dev/null 2>&1)"
//! sh:37
//! sh:38  for cmd pat in "$var[@]"; do
//! sh:39    if [[ $output = *$~pat* ]]; then
//! sh:40      (( $+opts[-r] )) && : ${(P)opts[-r]::=$cmd}
//! sh:41      _cmd_variant[$opts[-c]]="$cmd"
//! sh:42      return 0
//! sh:43    fi
//! sh:44  done
//! sh:45
//! sh:46  (( $+opts[-r] )) && : ${(P)opts[-r]::=$1}
//! sh:47  [[ $pre != builtin ]] && _cmd_variant[$opts[-c]]="$1"
//! sh:48
//! sh:49  return 1
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
