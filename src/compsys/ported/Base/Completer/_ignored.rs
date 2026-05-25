//! Port of `_ignored` from `Completion/Base/Completer/_ignored`.
//!
//! Full upstream body (68 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Use ignored matches.
//! sh: 4
//! sh: 5  [[ _matcher_num -gt 1 || $compstate[ignored] -eq 0 ]] && return 1
//! sh: 6
//! sh: 7  local comp
//! sh: 8  integer ind
//! sh: 9
//! sh:10  if ! zstyle -a ":completion:${curcontext}:" completer comp; then
//! sh:11    comp=( "${(@)_completers[1,_completer_num-1]}" )
//! sh:12    ind=${comp[(I)_ignored(|:*)]}
//! sh:13    (( ind )) && comp=("${(@)comp[ind,-1]}")
//! sh:14  fi
//! sh:15
//! sh:16  local _comp_no_ignore=yes tmp expl \
//! sh:17        _completer _completer_num \
//! sh:18        _matcher _c_matcher _matchers _matcher_num
//! sh:19
//! sh:20  _completer_num=1
//! sh:21
//! sh:22  for tmp in "$comp[@]"; do
//! sh:23    if [[ "$tmp" = *:-* ]]; then
//! sh:24      _completer="${${tmp%:*}[2,-1]//_/-}${tmp#*:}"
//! sh:25      tmp="${tmp%:*}"
//! sh:26    elif [[ $tmp = *:* ]]; then
//! sh:27      _completer="${tmp#*:}"
//! sh:28      tmp="${tmp%:*}"
//! sh:29    else
//! sh:30      _completer="${tmp[2,-1]//_/-}"
//! sh:31    fi
//! sh:32    curcontext="${curcontext/:[^:]#:/:${_completer}:}"
//! sh:33
//! sh:34    zstyle -a ":completion:${curcontext}:" matcher-list _matchers ||
//! sh:35        _matchers=( '' )
//! sh:36
//! sh:37    _matcher_num=1
//! sh:38    _matcher=''
//! sh:39    for _c_matcher in "$_matchers[@]"; do
//! sh:40      if [[ "$_c_matcher" == +* ]]; then
//! sh:41        _matcher="$_matcher $_c_matcher[2,-1]"
//! sh:42      else
//! sh:43        _matcher="$_c_matcher"
//! sh:44      fi
//! sh:45      if [[ "$tmp" != _ignored ]] && "$tmp"; then
//! sh:46        if zstyle -s ":completion:${curcontext}:" single-ignored tmp &&
//! sh:47           [[ $compstate[old_list] != shown &&
//! sh:48              $compstate[nmatches] -eq 1 ]]; then
//! sh:49          case "$tmp" in
//! sh:50          show) compstate[insert]='' compstate[list]='list force' tmp='' ;;
//! sh:51          menu)
//! sh:52            compstate[insert]=menu
//! sh:53            _description original expl original
//! sh:54            compadd "$expl[@]" -S '' - "$PREFIX$SUFFIX"
//! sh:55            ;;
//! sh:56          esac
//! sh:57        fi
//! sh:58
//! sh:59        return 0
//! sh:60      fi
//! sh:61
//! sh:62      (( _matcher_num++ ))
//! sh:63    done
//! sh:64
//! sh:65    (( _completer_num++ ))
//! sh:66  done
//! sh:67
//! sh:68  return 1
//! ```
//!
//! The shell version re-runs the preceding completers with
//! `_comp_no_ignore=yes` set so they emit the matches that had been
//! filtered out by `ignored-patterns` zstyle.
//!
//! Strict Rust port of the GATE half of the shell function. The
//! gating is the only piece the leaf layer can implement here:
//! re-running prior completers under `_comp_no_ignore=yes` is the
//! caller's job (it owns the completer dispatch loop). Returns
//! true iff the caller SHOULD run that loop now.
//!
//! Gate semantics (shell:5):
//! `[[ _matcher_num -gt 1 || $compstate[ignored] -eq 0 ]] && return 1`
//! → return false (don't run) when either we're past matcher 1 OR
//! there are no ignored matches to recover. Otherwise return true.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
