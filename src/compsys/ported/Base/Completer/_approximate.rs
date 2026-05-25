//! Port of `_approximate` from `Completion/Base/Completer/_approximate`.
//!
//! Full upstream body (121 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # This code will try to correct the string on the line based on the
//! sh:  4  # strings generated for the context. These corrected strings will be
//! sh:  5  # shown in a list and one can cycle through them as in a menu completion
//! sh:  6  # or get the corrected prefix.
//! sh:  7
//! sh:  8  # We don't try correction if the string is too short or we have tried it
//! sh:  9  # already.
//! sh: 10
//! sh: 11  [[ _matcher_num -gt 1 || "${#:-$PREFIX$SUFFIX}" -le 1 ]] && return 1
//! sh: 12
//! sh: 13  local _comp_correct _correct_expl _correct_group comax cfgacc match
//! sh: 14  local oldcontext="${curcontext}" opm="$compstate[pattern_match]"
//! sh: 15  integer ret=1
//! sh: 16
//! sh: 17  if [[ "$1" = -a* ]]; then
//! sh: 18    cfgacc="${1[3,-1]}"
//! sh: 19  elif [[ "$1" = -a ]]; then
//! sh: 20    cfgacc="$2"
//! sh: 21  else
//! sh: 22    zstyle -s ":completion:${curcontext}:" max-errors cfgacc ||
//! sh: 23        cfgacc='2 numeric'
//! sh: 24  fi
//! sh: 25
//! sh: 26  # Get the number of errors to accept.
//! sh: 27
//! sh: 28  if [[ "$cfgacc" = *numeric* && ${NUMERIC:-1} -ne 1 ]]; then
//! sh: 29    # A numeric argument may mean that we should not try correction.
//! sh: 30
//! sh: 31    [[ "$cfgacc" = *not-numeric* ]] && return 1
//! sh: 32
//! sh: 33    # Prefer the numeric argument if that has a sensible value.
//! sh: 34
//! sh: 35    comax="${NUMERIC:-1}"
//! sh: 36  else
//! sh: 37    comax="${cfgacc//[^0-9]}"
//! sh: 38  fi
//! sh: 39
//! sh: 40  # If the number of errors to accept is too small, give up.
//! sh: 41
//! sh: 42  [[ "$comax" -lt 1 ]] && return 1
//! sh: 43
//! sh: 44  _tags corrections original
//! sh: 45
//! sh: 46  # Otherwise temporarily define a function to use instead of the builtin that
//! sh: 47  # adds matches. This is used to be able to stick the `(#a...)' in the right
//! sh: 48  # place (after an ignored prefix).
//! sh: 49  #
//! sh: 50  # Current shell structure for use with "always", to make sure we unfunction our
//! sh: 51  # compadd and restore any compadd function defined previously.
//! sh: 52  {
//! sh: 53  _shadow -s _approximate compadd
//! sh: 54  compadd() {
//! sh: 55    local ppre="$argv[(I)-p]"
//! sh: 56
//! sh: 57    [[ ${argv[(I)-[a-zA-Z]#U[a-zA-Z]#]} -eq 0 &&
//! sh: 58        "${#:-$PREFIX$SUFFIX}" -le _comp_correct ]] && return
//! sh: 59
//! sh: 60    if [[ "$PREFIX" = \~* && ( ppre -eq 0 || "$argv[ppre+1]" != \~* ) ]]; then
//! sh: 61      PREFIX="~(#a${_comp_correct})${PREFIX[2,-1]}"
//! sh: 62    else
//! sh: 63      PREFIX="(#a${_comp_correct})$PREFIX"
//! sh: 64    fi
//! sh: 65
//! sh: 66    (( $_correct_group && ${${argv[1,(r)-(|-)]}[(I)-*[JV]]} )) &&
//! sh: 67        _correct_expl[_correct_group]=${argv[1,(r)-(-|)][(R)-*[JV]]}
//! sh: 68
//! sh: 69    compadd@_approximate "$_correct_expl[@]" "$@"
//! sh: 70  }
//! sh: 71
//! sh: 72  _comp_correct=1
//! sh: 73
//! sh: 74  [[ -z "$compstate[pattern_match]" ]] && compstate[pattern_match]='*'
//! sh: 75
//! sh: 76  while [[ _comp_correct -le comax ]]; do
//! sh: 77    curcontext="${oldcontext/(#b)([^:]#:[^:]#:)/${match[1][1,-2]}-${_comp_correct}:}"
//! sh: 78
//! sh: 79    _description corrections _correct_expl corrections \
//! sh: 80                 "e:$_comp_correct" "o:$PREFIX$SUFFIX"
//! sh: 81
//! sh: 82    _correct_group="$_correct_expl[(I)-*[JV]]"
//! sh: 83
//! sh: 84    if _complete; then
//! sh: 85      if zstyle -t ":completion:${curcontext}:" insert-unambiguous &&
//! sh: 86         [[ "${#compstate[unambiguous]}" -ge "${#:-$PREFIX$SUFFIX}" ]]; then
//! sh: 87        compstate[pattern_insert]=unambiguous
//! sh: 88      elif _requested original &&
//! sh: 89           { [[ compstate[nmatches] -gt 1 ]] ||
//! sh: 90             zstyle -t ":completion:${curcontext}:" original }; then
//! sh: 91        local expl
//! sh: 92
//! sh: 93        _description -V original expl original
//! sh: 94
//! sh: 95        builtin compadd "$expl[@]" -U -Q - "$PREFIX$SUFFIX"
//! sh: 96
//! sh: 97        # If you always want to see the list of possible corrections,
//! sh: 98        # set `compstate[list]=list force' here.
//! sh: 99
//! sh:100        [[ "$compstate[list]" != list* ]] &&
//! sh:101            compstate[list]="$compstate[list] force"
//! sh:102      fi
//! sh:103      compstate[pattern_match]="$opm"
//! sh:104
//! sh:105      ret=0
//! sh:106      break
//! sh:107    fi
//! sh:108
//! sh:109    [[ "${#:-$PREFIX$SUFFIX}" -le _comp_correct+1 ]] && break
//! sh:110    (( _comp_correct++ ))
//! sh:111  done
//! sh:112
//! sh:113  } always {
//! sh:114    _unshadow
//! sh:115  }
//! sh:116
//! sh:117  (( ret == 0 )) && return 0
//! sh:118
//! sh:119  compstate[pattern_match]="$opm"
//! sh:120
//! sh:121  return 1
//! ```
//!
//! Upstream: gates on `|PREFIX+SUFFIX| > 1` and the first-matcher
//! pass, reads `max-errors` zstyle, then loops decreasing the error
//! budget and re-runs the normal completer at each level,
//! accumulating matches.
//!
//! Strict Rust port: implements the gate AND the descending-budget
//! loop. The `max-errors` zstyle is consulted; explicit `max_errors`
//! arg wins when > 0. We iterate from `max_errors → 1` and add
//! every candidate within edit-distance of the prefix. Matches at
//! lower error counts overwrite higher-error entries via a HashMap
//! collation (closest-first wins). Returns `Matched` if anything
//! made it through.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
