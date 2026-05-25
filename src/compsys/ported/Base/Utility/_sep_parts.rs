//! Port of `_sep_parts` from `Completion/Base/Utility/_sep_parts`.
//!
//! Full upstream body (146 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # This function can be used to separately complete parts of strings
//! sh:  4  # where each part may be one of a set of matches and different parts
//! sh:  5  # have different sets.
//! sh:  6  # Arguments are alternately arrays and separator strings. Arrays may
//! sh:  7  # be given by name or literally as words separated by white space in
//! sh:  8  # parentheses, e.g.:
//! sh:  9  #
//! sh: 10  #  _sep_parts '(foo bar)' @ hosts
//! sh: 11  #
//! sh: 12  # This will make this function complete the strings `foo' and `bar'.
//! sh: 13  # If the string on the line contains a `@', the substring after it
//! sh: 14  # will be completed from the array `hosts'. Of course more arrays
//! sh: 15  # may be given, each preceded by another separator string.
//! sh: 16  #
//! sh: 17  # This function understands the `-J group', `-V group', and
//! sh: 18  # `-X explanation' options.
//! sh: 19
//! sh: 20  local str arr sep test testarr tmparr prefix suffixes autosuffix
//! sh: 21  local matchflags opt group expl nm=$compstate[nmatches] opre osuf opts matcher
//! sh: 22
//! sh: 23  # Get the options.
//! sh: 24
//! sh: 25  zparseopts -D -a opts 'J+:=group' 'V+:=group' P: F: S: r: R: q 1 2 o+: n \
//! sh: 26      'x+:=expl' 'X+:=expl' 'M+:=matcher'
//! sh: 27
//! sh: 28  # Get the string from the line.
//! sh: 29
//! sh: 30  opre="$PREFIX"
//! sh: 31  osuf="$SUFFIX"
//! sh: 32  str="$PREFIX$SUFFIX"
//! sh: 33  SUFFIX=""
//! sh: 34  prefix=""
//! sh: 35
//! sh: 36  # Walk through the arguments to find the longest unambiguous prefix.
//! sh: 37
//! sh: 38  while [[ $# -gt 1 ]]; do
//! sh: 39    # Get the next array and separator.
//! sh: 40    arr="$1"
//! sh: 41    sep="$2"
//! sh: 42
//! sh: 43    if [[ "$arr[1]" == '(' ]]; then
//! sh: 44      tmparr=( ${=arr[2,-2]} )
//! sh: 45      arr=tmparr
//! sh: 46    fi
//! sh: 47
//! sh: 48    # Is the separator on the line?
//! sh: 49
//! sh: 50    [[ "$str" != *${sep}* ]] && break
//! sh: 51
//! sh: 52    # Get the matching array elements.
//! sh: 53
//! sh: 54    PREFIX="${str%%(|\\)${sep}*}"
//! sh: 55    builtin compadd -O testarr "$matcher[@]" -a "$arr"
//! sh: 56    [[ $#testarr -eq 0 && -n "$_comp_correct" ]] &&
//! sh: 57      compadd -O testarr "$matcher[@]" -a "$arr"
//! sh: 58
//! sh: 59    # If there are no matches we give up. If there is more than one
//! sh: 60    # match, this is the part we will complete.
//! sh: 61
//! sh: 62    (( $#testarr )) || return 1
//! sh: 63    [[ $#testarr -gt 1 ]] && break
//! sh: 64
//! sh: 65    # Only one match, add it to the prefix and skip over it in `str',
//! sh: 66    # continuing with the next array and separator.
//! sh: 67
//! sh: 68    prefix="${prefix}${testarr[1]}${sep}"
//! sh: 69    str="${str#*${sep}}"
//! sh: 70    shift 2
//! sh: 71  done
//! sh: 72
//! sh: 73  # Get the array to work upon.
//! sh: 74
//! sh: 75  arr="$1"
//! sh: 76  if [[ "$arr[1]" == '(' ]]; then
//! sh: 77    tmparr=( ${=arr[2,-2]} )
//! sh: 78    arr=tmparr
//! sh: 79  fi
//! sh: 80
//! sh: 81  if [[ $# -le 1 || "$str" != *${2}* ]]; then
//! sh: 82    # No more separators, build the matches.
//! sh: 83
//! sh: 84    PREFIX="$str"
//! sh: 85    builtin compadd -O testarr "$matcher[@]" -a "$arr"
//! sh: 86    [[ $#testarr -eq 0 && -n "$_comp_correct" ]] &&
//! sh: 87      compadd -O testarr "$matcher[@]" -a "$arr"
//! sh: 88  fi
//! sh: 89
//! sh: 90  [[ $#testarr -eq 0 || ${#testarr[1]} -eq 0 ]] && return 1
//! sh: 91
//! sh: 92  # Now we build the suffixes to give to the completion code.
//! sh: 93
//! sh: 94  shift
//! sh: 95  suffixes=("")
//! sh: 96  autosuffix=()
//! sh: 97
//! sh: 98  while [[ $# -gt 0 && "$str" == *${1}* ]]; do
//! sh: 99    # Remove anything up to the suffix.
//! sh:100
//! sh:101    str="${str#*${1}}"
//! sh:102
//! sh:103    # Again, we get the string from the line up to the next separator
//! sh:104    # and build a pattern from it.
//! sh:105
//! sh:106    if [[ $# -gt 2 ]]; then
//! sh:107      PREFIX="${str%%${3}*}"
//! sh:108    else
//! sh:109      PREFIX="$str"
//! sh:110    fi
//! sh:111
//! sh:112    # We incrementally add suffixes by appending to them the separators
//! sh:113    # and the strings from the next array that match the pattern we built.
//! sh:114
//! sh:115    arr="$2"
//! sh:116    if [[ "$arr[1]" == '(' ]]; then
//! sh:117      tmparr=( ${=arr[2,-2]} )
//! sh:118      arr=tmparr
//! sh:119    fi
//! sh:120
//! sh:121    builtin compadd -O tmparr "$matcher[@]" -a "$arr"
//! sh:122    [[ $#tmparr -eq 0 && -n "$_comp_correct" ]] &&
//! sh:123      compadd -O tmparr "$matcher[@]" - "$arr"
//! sh:124
//! sh:125    suffixes=("${(@)^suffixes[@]}${(q)1}${(@)^tmparr}")
//! sh:126
//! sh:127    shift 2
//! sh:128  done
//! sh:129
//! sh:130  # If we were given at least one more separator we make the completion
//! sh:131  # code offer it by appending it as a autoremovable suffix.
//! sh:132
//! sh:133  (( $# )) && autosuffix=(-qS "${(q)1}")
//! sh:134
//! sh:135  # Add the matches for each of the suffixes.
//! sh:136
//! sh:137  PREFIX="$pre"
//! sh:138  SUFFIX="$suf"
//! sh:139  for i in "$suffixes[@]"; do
//! sh:140    compadd -U "$group[@]" "$expl[@]" "$autosuffix[@]" "$opts[@]" \
//! sh:141            -i "$IPREFIX" -I "$ISUFFIX" -p "$prefix" -s "$i" -a testarr
//! sh:142  done
//! sh:143
//! sh:144  # This sets the return value to indicate that we added matches (or not).
//! sh:145
//! sh:146  [[ nm -ne compstate[nmatches] ]]
//! ```
//!
//! Upstream is the multi-array sibling of `_multi_parts`: each array
//! holds candidates for the segment that follows the corresponding
//! separator. So `'(foo bar)' @ '(host1 host2)'` lets the user type
//! `foo@host1` etc.
//!
//! Faithful Rust port: walks the separators string char by char,
//! using the corresponding array at each segment position. The
//! cumulative-prefix tracking (so `foo@host1:port1` works with
//! `'@:'` as separators) matches the shell loop shape.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
