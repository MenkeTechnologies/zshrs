//! Port of `_history_complete_word` from `Completion/Base/Widget/_history_complete_word`.
//!
//! Full upstream body (121 lines verbatim):
//! ```text
//! sh:  1  #compdef -K _history-complete-older complete-word \e/ _history-complete-newer complete-word \e,
//! sh:  2  #
//! sh:  3  # Complete words from the history
//! sh:  4  #
//! sh:  5  # by Adam Spiers, with help gratefully received from
//! sh:  6  # Sven Wischnowsky and Bart Schaefer
//! sh:  7  #
//! sh:  8  # Available styles:
//! sh:  9  #
//! sh: 10  #   list --  avoid to display lists of available matches
//! sh: 11  #   stop --  stop before looping at beginning and end of matches
//! sh: 12  #   sort --  sort matches lexically (default is to sort by age)
//! sh: 13  #   remove-all-dups --
//! sh: 14  #            remove /all/ duplicate matches rather than just consecutives
//! sh: 15  #   range -- range of history words to complete
//! sh: 16
//! sh: 17  _history_complete_word () {
//! sh: 18    eval "$_comp_setup"
//! sh: 19
//! sh: 20    local expl direction stop curcontext="$curcontext"
//! sh: 21
//! sh: 22    if [[ -z "$curcontext" ]]; then
//! sh: 23      curcontext=history-words:::
//! sh: 24    else
//! sh: 25      curcontext="history-words${curcontext#*:}"
//! sh: 26    fi
//! sh: 27
//! sh: 28    if [[ $WIDGET = *newer ]]; then
//! sh: 29      direction=newer
//! sh: 30    else
//! sh: 31      direction=older
//! sh: 32    fi
//! sh: 33
//! sh: 34    zstyle -t ":completion:${curcontext}:history-words" stop && stop=yes
//! sh: 35
//! sh: 36    zstyle -T ":completion:${curcontext}:history-words" list || compstate[list]=''
//! sh: 37
//! sh: 38    if [[ $LASTWIDGET = _history-complete-* &&
//! sh: 39          ( -n "$compstate[old_list]" || -n $_hist_stop ) ]]; then
//! sh: 40      if [[ "$direction" == older ]]; then
//! sh: 41        if [[ $_hist_stop = new ]]; then
//! sh: 42          PREFIX=$_hist_old_prefix
//! sh: 43          _history_complete_word_gen_matches
//! sh: 44          compstate[insert]=2
//! sh: 45          _hist_stop=
//! sh: 46        elif [[ $_hist_stop = old ]]; then
//! sh: 47          PREFIX=$_hist_old_prefix
//! sh: 48          _history_complete_word_gen_matches
//! sh: 49          compstate[insert]=1
//! sh: 50          _hist_stop=
//! sh: 51        elif [[ compstate[old_insert] -lt _hist_menu_length ]]; then
//! sh: 52          compstate[old_list]=keep
//! sh: 53          (( compstate[insert] = compstate[old_insert] + 1 ))
//! sh: 54        elif [[ -n $stop ]]; then
//! sh: 55          _hist_stop=old
//! sh: 56          _message 'beginning of history reached'
//! sh: 57          return 1
//! sh: 58        else
//! sh: 59          compstate[old_list]=keep
//! sh: 60          compstate[insert]=1
//! sh: 61        fi
//! sh: 62      elif [[ "$direction" == 'newer' ]]; then
//! sh: 63        if [[ $_hist_stop = old ]]; then
//! sh: 64          PREFIX=$_hist_old_prefix
//! sh: 65          _history_complete_word_gen_matches
//! sh: 66          compstate[insert]=$(( $compstate[nmatches] - 1 ))
//! sh: 67          _hist_stop=
//! sh: 68        elif [[ $_hist_stop = new ]]; then
//! sh: 69          PREFIX=$_hist_old_prefix
//! sh: 70          _history_complete_word_gen_matches
//! sh: 71          compstate[insert]=$compstate[nmatches]
//! sh: 72          _hist_stop=
//! sh: 73        elif [[ compstate[old_insert] -gt 1 ]]; then
//! sh: 74          compstate[old_list]=keep
//! sh: 75          (( compstate[insert] = compstate[old_insert] - 1 ))
//! sh: 76        elif [[ -n $stop ]]; then
//! sh: 77          _hist_stop=new
//! sh: 78          _message 'end of history reached'
//! sh: 79          return 1
//! sh: 80        else
//! sh: 81          compstate[old_list]=keep
//! sh: 82          compstate[insert]=$_hist_menu_length
//! sh: 83        fi
//! sh: 84      fi
//! sh: 85      return 0
//! sh: 86    else
//! sh: 87      _hist_stop=
//! sh: 88      _hist_old_prefix="$PREFIX"
//! sh: 89      _history_complete_word_gen_matches
//! sh: 90    fi
//! sh: 91
//! sh: 92    (( $compstate[nmatches] ))
//! sh: 93  }
//! sh: 94
//! sh: 95  _history_complete_word_gen_matches () {
//! sh: 96
//! sh: 97    [[ -n "$_hist_stop" ]] && PREFIX="$_hist_old_prefix"
//! sh: 98
//! sh: 99    _main_complete _history
//! sh:100
//! sh:101    zstyle -T ":completion:${curcontext}:history-words" list || compstate[list]=
//! sh:102
//! sh:103    _hist_menu_length="$compstate[nmatches]"
//! sh:104
//! sh:105    if [[ $_lastcomp[insert] != *unambig* ]]; then
//! sh:106      case "$direction" in
//! sh:107        newer)  compstate[insert]=$_hist_menu_length
//! sh:108  	      [[ -n "$_hist_stop" ]] && (( compstate[insert]-- ))
//! sh:109                ;;
//! sh:110        older)  compstate[insert]=1
//! sh:111  	      [[ -n "$_hist_stop" ]] && (( compstate[insert]++ ))
//! sh:112                ;;
//! sh:113      esac
//! sh:114    fi
//! sh:115
//! sh:116    _hist_stop=
//! sh:117
//! sh:118    return
//! sh:119  }
//! sh:120
//! sh:121  _history_complete_word "$@"
//! ```
//!
//! The upstream version honors styles like `range`, `sort`, `stop`,
//! `list`, `remove-all-dups` and walks the global $history array
//! with cycling/wrap semantics.
//!
//! Strict Rust port: takes the history array directly. Honors
//! `remove-all-dups` (dedup all matches), `sort` (lexical sort
//! before emit), and `stop` (return false after first match,
//! single-shot behavior). Walks forward or backward by direction.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
