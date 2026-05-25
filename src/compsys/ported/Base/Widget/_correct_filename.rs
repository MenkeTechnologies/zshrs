//! Port of `_correct_filename` from `Completion/Base/Widget/_correct_filename`.
//!
//! Full upstream body (72 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xC
//! sh: 2
//! sh: 3  # Function to correct a filename.  Can be used as a completion widget,
//! sh: 4  # or as a function in its own right, in which case it will print the
//! sh: 5  # corrected filename to standard output.
//! sh: 6  #
//! sh: 7  # You can adapt max_approx to the maximum number of mistakes
//! sh: 8  # which are allowed in total.
//! sh: 9  #
//! sh:10  # If the numeric prefix is greater than 1, the maximum number of errors
//! sh:11  # will be set to that.
//! sh:12
//! sh:13  # Doesn't get right special characters in the filename; should
//! sh:14  # strip them (e.g. "foo\ bar" -> "foo bar") and then re-insert them.
//! sh:15
//! sh:16  emulate -LR zsh
//! sh:17  setopt extendedglob
//! sh:18
//! sh:19  local file="$PREFIX$SUFFIX" trylist tilde etilde testcmd
//! sh:20  integer approx max_approx=6
//! sh:21
//! sh:22  if [[ -z $WIDGET ]]; then
//! sh:23    file=$1
//! sh:24    local IPREFIX
//! sh:25  else
//! sh:26    (( ${NUMERIC:-1} > 1 )) && max_approx=$NUMERIC
//! sh:27  fi
//! sh:28
//! sh:29  if [[ $file = \~*/* ]]; then
//! sh:30    tilde=${file%%/*}
//! sh:31    etilde=${~tilde} 2>/dev/null
//! sh:32    file=${file/#$tilde/$etilde}
//! sh:33  fi
//! sh:34
//! sh:35  if [[ $CURRENT -eq 1 && $file != /* ]]; then
//! sh:36    testcmd=1
//! sh:37  elif [[ $file = \=* ]]; then
//! sh:38    [[ -n $WIDGET ]] && PREFIX="$PREFIX[2,-1]"
//! sh:39    IPREFIX="${IPREFIX}="
//! sh:40    file="$file[2,-1]"
//! sh:41    testcmd=1
//! sh:42  fi
//! sh:43
//! sh:44  # We need the -Q's to avoid the tilde we've put back getting quoted.
//! sh:45  if [[ -z $testcmd && -e "$file" ]] ||
//! sh:46    { [[ -n $testcmd ]] && whence "$file" >&/dev/null }; then
//! sh:47    if [[ -n $WIDGET ]]; then
//! sh:48      compadd -QUf -i "$IPREFIX" -I "$ISUFFIX" "${file/#$etilde/$tilde}"
//! sh:49      [[ -n "$compstate[insert]" ]] && compstate[insert]=menu
//! sh:50    else
//! sh:51      print "$file"
//! sh:52    fi
//! sh:53    return
//! sh:54  fi
//! sh:55
//! sh:56  for (( approx = 1; approx <= max_approx; approx++ )); do
//! sh:57    if [[ -z $testcmd ]]; then
//! sh:58      trylist=( (#a$approx)"$file"(N) )
//! sh:59    else
//! sh:60      trylist=( "${(@)${(@f)$(whence -wm "(#a$approx)$file" 2>/dev/null)}%:*}" )
//! sh:61      [[ $file = */* ]] || trylist=(${trylist##*/})
//! sh:62    fi
//! sh:63    (( $#trylist )) && break
//! sh:64  done
//! sh:65  (( $#trylist )) || return 1
//! sh:66
//! sh:67  if [[ -n $WIDGET ]]; then
//! sh:68    compadd -QUf -i "$IPREFIX" -I "$ISUFFIX" "${trylist[@]/#$etilde/$tilde}"
//! sh:69    [[ -n "$compstate[insert]" ]] && compstate[insert]=menu
//! sh:70  else
//! sh:71    print "$IPREFIX${^trylist[@]}"
//! sh:72  fi
//! ```
//!
//! Strict Rust port: handles `~/`/`~user/` expansion, walks the
//! parent directory, applies an INCREASING approximation budget
//! (1..=max_approx) and accepts on the first budget at which we
//! get any candidates. `max_approx` defaults to 6 (matching the
//! shell line 20) and can be overridden via `numeric_prefix` (>1).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
