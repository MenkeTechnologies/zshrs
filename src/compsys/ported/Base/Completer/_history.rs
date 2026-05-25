//! Port of `_history` from `Completion/Base/Completer/_history`.
//!
//! Full upstream body (65 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Hm, this *can* sensibly be used as a completer. But it could also be used
//! sh: 4  # as a utility function, so maybe it should be moved into another directory.
//! sh: 5  # Or maybe not. Hm.
//! sh: 6  #
//! sh: 7  #
//! sh: 8  # Complete words from the history
//! sh: 9  #
//! sh:10  # Code taken from _history_complete_words.
//! sh:11  #
//! sh:12  # Available styles:
//! sh:13  #
//! sh:14  #   sort --  sort matches lexically (default is to sort by age)
//! sh:15  #   remove-all-dups --
//! sh:16  #            remove /all/ duplicate matches rather than just consecutives
//! sh:17  #   range -- range of history words to complete
//! sh:18
//! sh:19  local opt expl max slice hmax=$#historywords beg=2
//! sh:20
//! sh:21  if zstyle -t ":completion:${curcontext}:" remove-all-dups; then
//! sh:22    opt=-
//! sh:23  else
//! sh:24    opt=-1
//! sh:25  fi
//! sh:26
//! sh:27  if zstyle -t ":completion:${curcontext}:" sort; then
//! sh:28    opt="${opt}J"
//! sh:29  else
//! sh:30    opt="${opt}V"
//! sh:31  fi
//! sh:32
//! sh:33  if zstyle -s ":completion:${curcontext}:" range max; then
//! sh:34    if [[ $max = *:* ]]; then
//! sh:35      slice=${max#*:}
//! sh:36      max=${max%:*}
//! sh:37    else
//! sh:38      slice=$max
//! sh:39    fi
//! sh:40    [[ max -gt hmax ]] && max=$hmax
//! sh:41  else
//! sh:42    max=$hmax
//! sh:43    slice=$max
//! sh:44  fi
//! sh:45
//! sh:46  PREFIX="$IPREFIX$PREFIX"
//! sh:47  IPREFIX=
//! sh:48  SUFFIX="$SUFFIX$ISUFFIX"
//! sh:49  ISUFFIX=
//! sh:50
//! sh:51  # We skip the first element of historywords so the current word doesn't
//! sh:52  # interfere with the completion
//! sh:53
//! sh:54  local -a hslice
//! sh:55  while [[ $compstate[nmatches] -eq 0 && beg -lt max ]]; do
//! sh:56    if [[ -n $compstate[quote] ]]
//! sh:57    then hslice=( ${(Q)historywords[beg,beg+slice]} )
//! sh:58    else hslice=( ${historywords[beg,beg+slice]} )
//! sh:59    fi
//! sh:60    _wanted "$opt" history-words expl 'history word' \
//! sh:61        compadd -Q -a hslice
//! sh:62    (( beg+=slice ))
//! sh:63  done
//! sh:64
//! sh:65  (( $compstate[nmatches] ))
//! ```
//!
//! Faithful Rust port: honors `HistoryOpts` for sort / range /
//! remove-all-dups / max-words knobs that mirror the corresponding
//! shell zstyles. The default opts match upstream defaults
//! (reverse iteration, full dedup, no range cap).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
