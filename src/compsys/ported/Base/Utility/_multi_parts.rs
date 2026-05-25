//! Port of `_multi_parts` from `Completion/Base/Utility/_multi_parts`.
//!
//! Full upstream body (261 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # This gets two arguments, a separator (which should be only one
//! sh:  4  # character) and an array. As usual, the array may be given by it's
//! sh:  5  # name or literal as in `(foo bar baz)' (words separated by spaces in
//! sh:  6  # parentheses).
//! sh:  7  # The parts of words from the array that are separated by the
//! sh:  8  # separator character are then completed independently.
//! sh:  9
//! sh: 10  local sep pref npref i tmp2 group expl menu pre suf opre osuf orig cpre
//! sh: 11  local opts sopts matcher imm
//! sh: 12  typeset -U tmp1 matches
//! sh: 13
//! sh: 14  # Get the options.
//! sh: 15
//! sh: 16  zparseopts -D -a sopts \
//! sh: 17      'J+:=group' 'V+:=group' 'x+:=expl' 'X+:=expl' 'P:=opts' 'F:=opts' \
//! sh: 18      S: r: R: q 1 2 o+: n 'f=opts' 'M+:=matcher' 'i=imm'
//! sh: 19
//! sh: 20  sopts=( "$sopts[@]" "$opts[@]" )
//! sh: 21  if (( $#matcher )); then
//! sh: 22    matcher="${matcher[2]}"
//! sh: 23  else
//! sh: 24    matcher=
//! sh: 25  fi
//! sh: 26
//! sh: 27  # Get the arguments, first the separator, then the array. The array is
//! sh: 28  # stored in `tmp1'. Further on the array `matches' will always contain
//! sh: 29  # those words from the original array that still match everything we have
//! sh: 30  # tried to match while we walk through the string from the line.
//! sh: 31
//! sh: 32  sep="$1"
//! sh: 33  if [[ "${2[1]}" = '(' ]]; then
//! sh: 34    tmp1=( ${=2[2,-2]} )
//! sh: 35  else
//! sh: 36    tmp1=( "${(@P)2}" )
//! sh: 37  fi
//! sh: 38
//! sh: 39  # In `pre' and `suf' we will hold the prefix and the suffix from the
//! sh: 40  # line while we walk through them. The original string are used
//! sh: 41  # temporarily for matching.
//! sh: 42
//! sh: 43  pre="$PREFIX"
//! sh: 44  suf="$SUFFIX"
//! sh: 45  opre="$PREFIX"
//! sh: 46  osuf="$SUFFIX"
//! sh: 47  orig="$PREFIX$SUFFIX"
//! sh: 48
//! sh: 49  # Special handling for menu completion?
//! sh: 50
//! sh: 51  [[ $compstate[insert] = (*menu|[0-9]*) || -n "$_comp_correct" ||
//! sh: 52     ( $#compstate[pattern_match] -ne 0 &&
//! sh: 53       "$orig" != "${orig:q}" ) ]] && menu=yes
//! sh: 54
//! sh: 55  # In `pref' we collect the unambiguous prefix path.
//! sh: 56
//! sh: 57  pref=''
//! sh: 58
//! sh: 59  # If the string from the line matches at least one of the strings,
//! sh: 60  # we use only the matching strings.
//! sh: 61
//! sh: 62  compadd -O matches -M "r:|${sep}=* r:|=* $matcher" -a tmp1
//! sh: 63
//! sh: 64  (( $#matches )) || matches=( "$tmp1[@]" )
//! sh: 65
//! sh: 66  while true; do
//! sh: 67
//! sh: 68    # Get the prefix and suffix for matching.
//! sh: 69
//! sh: 70    if [[ "$pre" = *${sep}* ]]; then
//! sh: 71      PREFIX="${pre%%${sep}*}"
//! sh: 72      SUFFIX=""
//! sh: 73    else
//! sh: 74      PREFIX="${pre}"
//! sh: 75      SUFFIX="${suf%%${sep}*}"
//! sh: 76    fi
//! sh: 77
//! sh: 78    # Check if the component for some of the possible matches is equal
//! sh: 79    # to the string from the line. If there are such strings, we directly
//! sh: 80    # use the stuff from the line. This avoids having `foo' complete to
//! sh: 81    # both `foo' and `foobar'.
//! sh: 82
//! sh: 83    if [[ -n "$PREFIX$SUFFIX" || "$pre" = ${sep}* ]]; then
//! sh: 84      tmp1=( "${(@M)matches:#${PREFIX}${SUFFIX}${sep}*}" )
//! sh: 85    else
//! sh: 86      tmp1=()
//! sh: 87    fi
//! sh: 88
//! sh: 89    if (( $#tmp1 )); then
//! sh: 90      npref="${PREFIX}${SUFFIX}${sep}"
//! sh: 91    else
//! sh: 92      # No exact match, see how many strings match what's on the line.
//! sh: 93
//! sh: 94      builtin compadd -O tmp1 -M "r:|${sep}=* r:|=* $matcher" - "${(@)${(@)matches%%${sep}*}:#}"
//! sh: 95
//! sh: 96      [[ $#tmp1 -eq 0 && -n "$_comp_correct" ]] &&
//! sh: 97        compadd -O tmp1 -M "r:|${sep}=* r:|=* $matcher" - "${(@)${(@)matches%%${sep}*}:#}"
//! sh: 98
//! sh: 99      if [[ $#tmp1 -eq 1 ]]; then
//! sh:100
//! sh:101        # Only one match. If there are still separators from the line
//! sh:102        # we just accept this component. Otherwise we insert what we
//! sh:103        # have collected, probably giving it a separator character
//! sh:104        # as a suffix.
//! sh:105
//! sh:106        if [[ "$pre$suf" = *${sep}* ]]; then
//! sh:107          npref="${tmp1[1]}${sep}"
//! sh:108        else
//! sh:109          matches=( "${(@M)matches:#${tmp1[1]}*}" )
//! sh:110
//! sh:111  	PREFIX="${cpre}${pre}"
//! sh:112  	SUFFIX="$suf"
//! sh:113
//! sh:114  	if [[ $#imm -ne 0 && $#matches -eq 1 ]] ||
//! sh:115             zstyle -t ":completion:${curcontext}:" expand suffix; then
//! sh:116  	  compadd "$group[@]" "$expl[@]" "$sopts[@]" \
//! sh:117                    -M "r:|${sep}=* r:|=* $matcher" - $pref$matches
//! sh:118          else
//! sh:119  	  if (( $matches[(I)${tmp1[1]}${sep}*] )); then
//! sh:120  	    compadd "$group[@]" "$expl[@]" -p "$pref" -r "$sep" -S "$sep" "$opts[@]" \
//! sh:121                      -M "r:|${sep}=* r:|=* $matcher" - "$tmp1[1]"
//! sh:122            else
//! sh:123  	    compadd "$group[@]" "$expl[@]" -p "$pref" "$sopts[@]" \
//! sh:124                      -M "r:|${sep}=* r:|=* $matcher" - "$tmp1[1]"
//! sh:125            fi
//! sh:126          fi
//! sh:127  	return
//! sh:128        fi
//! sh:129      elif (( $#tmp1 )); then
//! sh:130        local ret=1
//! sh:131
//! sh:132        # More than one match. First we get all strings that match the
//! sh:133        # rest from the line.
//! sh:134
//! sh:135        PREFIX="$pre"
//! sh:136        SUFFIX="$suf"
//! sh:137        compadd -O matches -M "r:|${sep}=* r:|=* $matcher" -a matches
//! sh:138
//! sh:139        if [[ "$pre" = *${sep}* ]]; then
//! sh:140   	PREFIX="${cpre}${pre%%${sep}*}"
//! sh:141  	SUFFIX="${sep}${pre#*${sep}}${suf}"
//! sh:142        else
//! sh:143          PREFIX="${cpre}${pre}"
//! sh:144  	SUFFIX="$suf"
//! sh:145        fi
//! sh:146
//! sh:147        # The purpose of this check (or one purpose, anyway) seems to be to ensure
//! sh:148        # that the suffix for the current segment on the command line doesn't
//! sh:149        # match across segments. For example, we want $matches for a<TAB>c to
//! sh:150        # include abc/d, but not abd/c. If we don't have anything on the command
//! sh:151        # line for this segment, though, we can skip it. (The difference is only
//! sh:152        # noticeable when there are a huge number of possibilities)
//! sh:153        [[ -n $pre$suf ]] &&
//! sh:154        matches=( ${(@M)matches:#(${(j<|>)~${(@b)tmp1}})*} )
//! sh:155
//! sh:156        if ! zstyle -t ":completion:${curcontext}:" expand suffix ||
//! sh:157           [[ -n "$menu" || -z "$compstate[insert]" ]]; then
//! sh:158
//! sh:159          # With menu completion we add only the ambiguous component with
//! sh:160          # the prefix collected and a separator for the matches that
//! sh:161          # have more components.
//! sh:162
//! sh:163          tmp2="$pre$suf"
//! sh:164          if [[ "$tmp2" = *${sep}* ]]; then
//! sh:165            tmp2=(-s "${sep}${tmp2#*${sep}}")
//! sh:166          else
//! sh:167  	  tmp2=()
//! sh:168          fi
//! sh:169
//! sh:170
//! sh:171          compadd "$group[@]" "$expl[@]" -r "$sep" -S "$sep" "$opts[@]" \
//! sh:172  	        -p "$pref" "$tmp2[@]" -M "r:|${sep}=* r:|=* $matcher" - \
//! sh:173                  "${(@)${(@)${(@M)matches:#*${sep}}%%${sep}*}:#}" && ret=0
//! sh:174          (( $matches[(I)${sep}*] )) &&
//! sh:175              compadd "$group[@]" "$expl[@]" -S '' "$opts[@]" \
//! sh:176  	            -p "$pref" \
//! sh:177                      -M "r:|${sep}=* r:|=* $matcher" - "$sep" && ret=0
//! sh:178          compadd "$group[@]" "$expl[@]" -r "$sep" -S "$sep" "$opts[@]" \
//! sh:179                  -p "$pref" "$tmp2[@]" -M "r:|${sep}=* r:|=* $matcher" - \
//! sh:180                  "${(@)${(@)${(@M)matches:#*?${sep}?*}%%${sep}*}:#}" && ret=0
//! sh:181          compadd "$group[@]" "$expl[@]" -S '' "$opts[@]" -p "$pref" "$tmp2[@]" \
//! sh:182                  -M "r:|${sep}=* r:|=* $matcher" - \
//! sh:183                  "${(@)matches:#*${sep}*}" && ret=0
//! sh:184        else
//! sh:185          # With normal completion we add all matches one-by-one with
//! sh:186  	# the unmatched part as a suffix. This will insert the longest
//! sh:187  	# unambiguous string for all matching strings.
//! sh:188
//! sh:189          compadd "$group[@]" "$expl[@]" "$opts[@]" \
//! sh:190  	        -p "$pref" -s "${i#*${sep}}" \
//! sh:191                  -M "r:|${sep}=* r:|=* $matcher" - \
//! sh:192                  "${(@)${(@)${(@M)matches:#*${sep}*}%%${sep}*}:#}" && ret=0
//! sh:193          compadd "$group[@]" "$expl[@]" -S '' "$opts[@]" -p "$pref" \
//! sh:194                  -M "r:|${sep}=* r:|=* $matcher" - \
//! sh:195                  "${(@)matches:#*${sep}*}" && ret=0
//! sh:196        fi
//! sh:197        return ret
//! sh:198      else
//! sh:199        # We are here if no string matched what's on the line. In this
//! sh:200        # case we insert the expanded prefix we collected if it differs
//! sh:201        # from the original string from the line.
//! sh:202
//! sh:203        { ! zstyle -t ":completion:${curcontext}:" expand prefix ||
//! sh:204          [[ "$orig" = "$pref$pre$suf" ]] } && return 1
//! sh:205
//! sh:206        PREFIX="${cpre}${pre}"
//! sh:207        SUFFIX="$suf"
//! sh:208
//! sh:209        if [[ -n "$suf" ]]; then
//! sh:210          compadd "$group[@]" "$expl[@]" -s "$suf" "$sopts[@]" \
//! sh:211                  -M "r:|${sep}=* r:|=* $matcher" - "$pref$pre"
//! sh:212        else
//! sh:213          compadd "$group[@]" "$expl[@]" -S '' "$opts[@]" \
//! sh:214                  -M "r:|${sep}=* r:|=* $matcher" - "$pref$pre"
//! sh:215        fi
//! sh:216        return
//! sh:217      fi
//! sh:218    fi
//! sh:219
//! sh:220    # We just accepted and/or expanded a component from the line. We
//! sh:221    # remove it from the matches (using only those that have a least
//! sh:222    # the skipped string) and ad it the `pref'.
//! sh:223
//! sh:224    matches=( "${(@)${(@)${(@M)matches:#${npref}*}#*${sep}}:#}" )
//! sh:225    pref="$pref$npref"
//! sh:226
//! sh:227    # Now we set `pre' and `suf' to their new values.
//! sh:228
//! sh:229    if [[ "$pre" = *${sep}* ]]; then
//! sh:230      cpre="${cpre}${pre%%${sep}*}${sep}"
//! sh:231      pre="${pre#*${sep}}"
//! sh:232    elif [[ "$suf" = *${sep}* ]]; then
//! sh:233      cpre="${cpre}${pre}${suf%%${sep}*}${sep}"
//! sh:234      pre="${suf#*${sep}}"
//! sh:235      suf=""
//! sh:236    else
//! sh:237      # The string from the line is fully handled. If we collected an
//! sh:238      # unambiguous prefix and that differs from the original string,
//! sh:239      # we insert it.
//! sh:240
//! sh:241      PREFIX="${opre}${osuf}"
//! sh:242      SUFFIX=""
//! sh:243
//! sh:244      if [[ -n "$pref" && "$orig" != "$pref" ]]; then
//! sh:245        if [[ "$pref" = *${sep}*${sep} ]]; then
//! sh:246          compadd "$group[@]" "$expl[@]" "$opts[@]" \
//! sh:247                  -p "${pref%${sep}*${sep}}${sep}" -S "$sep" \
//! sh:248                  -M "r:|${sep}=* r:|=* $matcher" - "${${pref%${sep}}##*${sep}}"
//! sh:249
//! sh:250        elif [[ "$pref" = *${sep}* ]]; then
//! sh:251          compadd "$group[@]" "$expl[@]" -S '' "$opts[@]" \
//! sh:252                  -p "${pref%${sep}*}${sep}" \
//! sh:253                  -M "r:|${sep}=* r:|=* $matcher" - "${pref##*${sep}}"
//! sh:254        else
//! sh:255          compadd "$group[@]" "$expl[@]" -S '' "$opts[@]" \
//! sh:256                  -M "r:|${sep}=* r:|=* $matcher" - "$pref"
//! sh:257        fi
//! sh:258      fi
//! sh:259      return
//! sh:260    fi
//! sh:261  done
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
