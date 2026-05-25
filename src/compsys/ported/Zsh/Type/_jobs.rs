//! Port of `_jobs` from `Completion/Zsh/Type/_jobs`.
//!
//! Full upstream body (84 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl disp jobs job jids pfx='%' desc how expls sep
//! sh: 4
//! sh: 5  if [[ "$1" = -t ]]; then
//! sh: 6    zstyle -T ":completion:${curcontext}:jobs" prefix-needed &&
//! sh: 7        [[ "$PREFIX" != %* && compstate[nmatches] -ne 0 ]] && return 1
//! sh: 8    shift
//! sh: 9  fi
//! sh:10  zstyle -t ":completion:${curcontext}:jobs" prefix-hidden && pfx=''
//! sh:11  zstyle -T ":completion:${curcontext}:jobs" verbose       && desc=yes
//! sh:12
//! sh:13  if [[ "$1" = -r ]]; then
//! sh:14    jids=( "${(@k)jobstates[(R)running*]}" )
//! sh:15    shift
//! sh:16    expls='running job'
//! sh:17  elif [[ "$1" = -s ]]; then
//! sh:18    jids=( "${(@k)jobstates[(R)suspended*]}" )
//! sh:19    shift
//! sh:20    expls='suspended job'
//! sh:21  else
//! sh:22    [[ "$1" = - ]] && shift
//! sh:23    jids=( "${(@k)jobtexts}" )
//! sh:24    expls=job
//! sh:25  fi
//! sh:26
//! sh:27  if [[ -n "$desc" ]]; then
//! sh:28    disp=()
//! sh:29    zstyle -s ":completion:${curcontext}:jobs" list-separator sep || sep=--
//! sh:30    for job in "$jids[@]"; do
//! sh:31      [[ -n "$desc" ]] &&
//! sh:32          disp=( "$disp[@]" "${pfx}${(r:2:: :)job} $sep ${(r:COLUMNS-8:: :)jobtexts[$job]}" )
//! sh:33    done
//! sh:34  fi
//! sh:35
//! sh:36  zstyle -s ":completion:${curcontext}:jobs" numbers how
//! sh:37
//! sh:38  if [[ "$how" = (yes|true|on|1) ]]; then
//! sh:39    jobs=( "$jids[@]" )
//! sh:40  else
//! sh:41    local texts i text str tmp num max=0
//! sh:42
//! sh:43    # Find shortest unambiguous strings.
//! sh:44
//! sh:45    texts=( "$jobtexts[@]" )
//! sh:46    jobs=()
//! sh:47    for i in "$jids[@]"; do
//! sh:48      text="$jobtexts[$i]"
//! sh:49      str="${text%% *}"
//! sh:50      if [[ "$text" = *\ * ]]; then
//! sh:51        text="${text#* }"
//! sh:52      else
//! sh:53        text=""
//! sh:54      fi
//! sh:55      tmp=( "${(@M)texts:#${str}*}" )
//! sh:56      num=1
//! sh:57      while [[ -n "$text" && $#tmp -ge 2 ]]; do
//! sh:58        str="${str} ${text%% *}"
//! sh:59        if [[ "$text" = *\ * ]]; then
//! sh:60          text="${text#* }"
//! sh:61        else
//! sh:62          text=""
//! sh:63        fi
//! sh:64        tmp=( "${(@M)texts:#${str}*}" )
//! sh:65        (( num++ ))
//! sh:66      done
//! sh:67
//! sh:68      [[ num -gt max ]] && max="$num"
//! sh:69
//! sh:70      jobs=( "$jobs[@]" "$str" )
//! sh:71    done
//! sh:72
//! sh:73    if [[ "$how" = [0-9]## && max -gt how ]]; then
//! sh:74      jobs=( "$jids[@]" )
//! sh:75    else
//! sh:76      [[ -z "$pfx" && -n "$desc" ]] && disp=( "${(@)disp#%}" )
//! sh:77    fi
//! sh:78  fi
//! sh:79
//! sh:80  if [[ -n "$desc" ]]; then
//! sh:81    _wanted jobs expl "$expls" compadd "$@" -ld disp - "%$^jobs[@]"
//! sh:82  else
//! sh:83    _wanted jobs expl "$expls" compadd "$@" - "%$^jobs[@]"
//! sh:84  fi
//! ```
//!
//! Strict Rust port: caller injects the live `jobtexts`/`jobstates`
//! tables. Job IDs come back prefixed with `%` (`pfx`) unless the
//! `prefix-hidden` style is truthy.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
