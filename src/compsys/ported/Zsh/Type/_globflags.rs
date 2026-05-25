//! Port of `_globflags` from `Completion/Zsh/Type/_globflags`.
//!
//! Full upstream body (62 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete 'globbing flags', i.e., '(#x)'; everything up to the '#' will
//! sh: 4  # have been "compset -P"'d by the caller.
//! sh: 5
//! sh: 6  local ret=1
//! sh: 7  local -a flags
//! sh: 8  local preprefix=$IPREFIX
//! sh: 9
//! sh:10  compset -P '([ilIUubBmMcq]|a(|<->))##'
//! sh:11  # make sure to not consider anything before the '#'
//! sh:12  preprefix=${IPREFIX[$#preprefix,-1]}
//! sh:13  if [[ $preprefix = *\#q* ]]; then
//! sh:14    _globquals
//! sh:15    return
//! sh:16  elif [[ $preprefix = *q* ]]; then
//! sh:17    _message 'q flag has to be specified by itself'
//! sh:18    return
//! sh:19  elif [[ $preprefix = *a(|<->) ]]; then
//! sh:20    _message -e number 'errors'
//! sh:21    if [[ $preprefix = *a ]]; then
//! sh:22      return
//! sh:23    else
//! sh:24      compset -P '<->'
//! sh:25    fi
//! sh:26  elif [[ $preprefix = *\#c ]]; then
//! sh:27    _message -e range 'repetitions (min,max) or (exact)'
//! sh:28    return
//! sh:29  fi
//! sh:30
//! sh:31  flags=(
//! sh:32    'i:case insensitive'
//! sh:33    'l:lower case characters match uppercase'
//! sh:34    'I:case sensitive matching'
//! sh:35    's:match start of string'
//! sh:36    'e:match end of string'
//! sh:37    'U:consider all characters to be one byte'
//! sh:38    'u:support multibyte characters in pattern'
//! sh:39  )
//! sh:40  [[ $compstate[context] = condition ]] && flags+=(
//! sh:41    'b:activate backreferences'
//! sh:42    'B:deactivate backreferences'
//! sh:43    'm:set reference to entire matched data'
//! sh:44    'M:deactivate m flag'
//! sh:45  )
//! sh:46  flags=( ${flags:#[$preprefix[(R)\#,-1]]*} )
//! sh:47  if [[ $IPREFIX != *'#' ]]; then
//! sh:48    flags=( ${flags:#[se]*} )
//! sh:49  fi
//! sh:50  _describe -t globflags "glob flag" flags -Q -S ')' && ret=0
//! sh:51  flags=(
//! sh:52    'a:approximate matching'
//! sh:53    'q:introduce glob qualifier'
//! sh:54    'c:match repetitions of preceding pattern'
//! sh:55  )
//! sh:56  flags=( ${flags:#[$preprefix[(R)\#,-1]]*} )
//! sh:57  if [[ $IPREFIX != *'#' ]]; then
//! sh:58    flags=( ${flags:#[cq]*} )
//! sh:59  fi
//! sh:60  _describe -t globflags "glob flag" flags -Q -S '' && ret=0
//! sh:61
//! sh:62  return ret
//! ```
//!
//! Glob flag chars (zsh):
//! i,I — case insensitive / case insensitive in pattern
//! l   — lowercase
//! u   — uppercase
//! b   — backreferences
//! B   — disable backreferences
//! m,M — Match local/global
//! c   — count
//! q   — quote (must be standalone)
//! a   — approximate (optional count: `a3`)
//!
//! Strict Rust port: emit the glob flag letters as completion
//! candidates. The `q` standalone constraint is enforced — if the
//! preceding glob-flag prefix already has flags, `q` is dropped.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
