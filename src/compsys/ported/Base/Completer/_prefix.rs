//! Port of `_prefix` from `Completion/Base/Completer/_prefix`.
//!
//! Full upstream body (62 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Try to ignore the suffix. A bit like e-o-c-prefix.
//! sh: 4
//! sh: 5  [[ _matcher_num -gt 1 || -z "$SUFFIX" ]] && return 1
//! sh: 6
//! sh: 7  local comp curcontext="$curcontext" tmp suf="$SUFFIX" \
//! sh: 8        _completer \
//! sh: 9        _matcher _c_matcher _matchers _matcher_num
//! sh:10  integer ind
//! sh:11
//! sh:12  if ! zstyle -a ":completion:${curcontext}:" completer comp; then
//! sh:13    comp=( "${(@)_completers[1,_completer_num-1]}" )
//! sh:14    ind=${comp[(I)_prefix(|:*)]}
//! sh:15    (( ind )) && comp=("${(@)comp[ind,-1]}")
//! sh:16  fi
//! sh:17
//! sh:18  if zstyle -t ":completion:${curcontext}:" add-space; then
//! sh:19    ISUFFIX=" $SUFFIX"
//! sh:20  else
//! sh:21    ISUFFIX="$SUFFIX"
//! sh:22  fi
//! sh:23  SUFFIX=''
//! sh:24
//! sh:25  local _completer_num=1
//! sh:26
//! sh:27  for tmp in "$comp[@]"; do
//! sh:28    if [[ "$tmp" = *:-* ]]; then
//! sh:29      _completer="${${tmp%:*}[2,-1]//_/-}${tmp#*:}"
//! sh:30      tmp="${tmp%:*}"
//! sh:31    elif [[ $tmp = *:* ]]; then
//! sh:32      _completer="${tmp#*:}"
//! sh:33      tmp="${tmp%:*}"
//! sh:34    else
//! sh:35      _completer="${tmp[2,-1]//_/-}"
//! sh:36    fi
//! sh:37    curcontext="${curcontext/:[^:]#:/:${_completer}:}"
//! sh:38
//! sh:39    zstyle -a ":completion:${curcontext}:" matcher-list _matchers ||
//! sh:40        _matchers=( '' )
//! sh:41
//! sh:42    _matcher_num=1
//! sh:43    _matcher=''
//! sh:44    for _c_matcher in "$_matchers[@]"; do
//! sh:45      if [[ "$_c_matcher" == +* ]]; then
//! sh:46        _matcher="$_matcher $_c_matcher[2,-1]"
//! sh:47      else
//! sh:48        _matcher="$_c_matcher"
//! sh:49      fi
//! sh:50
//! sh:51      if [[ "$tmp" != _prefix ]] && "$tmp"; then
//! sh:52        if [[ -n $compstate[old_list] || ${compstate[unambiguous]%$suf} == $PREFIX ]]; then
//! sh:53          compstate[to_end]=match
//! sh:54        fi
//! sh:55        return 0
//! sh:56      fi
//! sh:57      (( _matcher_num++ ))
//! sh:58    done
//! sh:59    (( _completer_num++ ))
//! sh:60  done
//! sh:61
//! sh:62  return 1
//! ```
//!
//! The shell version moves SUFFIX into ISUFFIX (the "ignored suffix",
//! preserved on the line but excluded from completion matching),
//! then runs the rest of the completer pipeline against bare PREFIX.
//!
//! Strict Rust port: honors `matcher_num > 1 || empty SUFFIX → bail`
//! gate AND the `add-space` style. Moves SUFFIX into ISUFFIX (with
//! optional leading space) for the action's duration, clears SUFFIX,
//! then restores both on return.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
