//! Port of `_complete_debug` from `Completion/Base/Widget/_complete_debug`.
//!
//! Full upstream body (40 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-x?
//! sh: 2
//! sh: 3  eval "$_comp_setup"
//! sh: 4
//! sh: 5  (( $+_debug_count )) || integer -g _debug_count
//! sh: 6  local tmp=${TMPPREFIX}${$}${words[1]:t}$[++_debug_count]
//! sh: 7  local pager w="${(qq)words}"
//! sh: 8
//! sh: 9  integer debug_fd=-1
//! sh:10  {
//! sh:11    if [[ -t 2 ]]; then
//! sh:12      zmodload -F zsh/files b:zf_ln 2>/dev/null &&
//! sh:13      zf_ln -fn =(<<<'') $tmp &&
//! sh:14      exec {debug_fd}>&2 2>| $tmp
//! sh:15    fi
//! sh:16
//! sh:17    local -a debug_indent
//! sh:18    () {
//! sh:19      setopt localoptions no_ignorebraces
//! sh:20      debug_indent=( '%'{3..20}'(e. .)' )
//! sh:21    }
//! sh:22    local PROMPT4="$PROMPT4" PS4="${(j::)debug_indent}+%N:%i> "
//! sh:23    setopt xtrace
//! sh:24    : $ZSH_NAME $ZSH_VERSION
//! sh:25    ${1:-_main_complete}
//! sh:26    integer ret=$?
//! sh:27    unsetopt xtrace
//! sh:28
//! sh:29    if (( debug_fd != -1 )); then
//! sh:30      zstyle -s ':completion:complete-debug::::' pager pager
//! sh:31      print -sR "${pager:-${PAGER:-${VISUAL:-${EDITOR:-more}}}} ${(q)tmp} ;: $w"
//! sh:32      _message -r "Trace output left in $tmp (up-history to view)"
//! sh:33      if [[ $compstate[nmatches] -le 1 && $compstate[list] != *force* ]]; then
//! sh:34          compstate[list]='list force messages'
//! sh:35      fi
//! sh:36    fi
//! sh:37  } always {
//! sh:38    (( debug_fd != -1 )) && exec 2>&$debug_fd {debug_fd}>&-
//! sh:39  }
//! sh:40  return ret
//! ```
//!
//! Faithful Rust port: writes a structured diagnostic dump to a
//! temp file (mtime-stamped so multiple runs don't collide) AND
//! returns the path via a side-channel so callers can surface it
//! to the user. Always returns `NoMatch` because the widget's
//! purpose is to PRINT state, not emit completions.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
