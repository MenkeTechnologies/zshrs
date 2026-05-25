//! Port of `_brace_parameter` from `Completion/Zsh/Context/_brace_parameter`.
//!
//! Full upstream body (214 lines verbatim):
//! ```text
//! sh:  1  #compdef -brace-parameter-
//! sh:  2
//! sh:  3  local char delim found_percent found_m exp
//! sh:  4  local -a flags
//! sh:  5  integer q_last n_q
//! sh:  6
//! sh:  7  if [[ $PREFIX = *'${('[^\)]# ]]; then
//! sh:  8    # Parameter flags.
//! sh:  9    compset -p 3
//! sh: 10
//! sh: 11    # Based on code in _globquals.
//! sh: 12    while [[ -n $PREFIX ]]; do
//! sh: 13      char=$PREFIX[1]
//! sh: 14      compset -p 1
//! sh: 15      if [[ $char = q ]]; then
//! sh: 16        (( q_last++, n_q++ ))
//! sh: 17        continue
//! sh: 18      else
//! sh: 19        (( q_last = 0 ))
//! sh: 20      fi
//! sh: 21      # Skip arguments to find what's left to complete
//! sh: 22      case $char in
//! sh: 23        (%)
//! sh: 24        found_percent=1
//! sh: 25        ;;
//! sh: 26
//! sh: 27        (m)
//! sh: 28        found_m=1
//! sh: 29        ;;
//! sh: 30
//! sh: 31        ([gIjsZ_])
//! sh: 32        # Single delimited argument.
//! sh: 33        if [[ -z $PREFIX ]]; then
//! sh: 34  	_delimiters qualifier-$char
//! sh: 35  	return
//! sh: 36        elif ! _globqual_delims; then
//! sh: 37  	# still completing argument
//! sh: 38  	case $char in
//! sh: 39  	  (g)
//! sh: 40  	  compset -P '*'
//! sh: 41  	  flags=('o:octal escapes' 'c:expand ^X etc.' 'e:expand \\M-t etc.')
//! sh: 42  	  _describe -t format 'format option' flags -Q -S ''
//! sh: 43  	  ;;
//! sh: 44
//! sh: 45  	  (I)
//! sh: 46  	  _message 'integer expression'
//! sh: 47  	  ;;
//! sh: 48
//! sh: 49  	  ([js])
//! sh: 50  	  _message "separator"
//! sh: 51  	  ;;
//! sh: 52
//! sh: 53  	  (Z)
//! sh: 54  	  compset -P '*'
//! sh: 55  	  flags=(
//! sh: 56  	    'c:parse comments as strings (else as ordinary words)'
//! sh: 57  	    'C:strip comments (else treat as ordinary words)'
//! sh: 58  	    'n:treat newlines as whitespace'
//! sh: 59  	  )
//! sh: 60  	  _describe -t format 'format option' flags -Q -S ''
//! sh: 61  	  ;;
//! sh: 62
//! sh: 63  	  (_)
//! sh: 64  	  _message "no useful values"
//! sh: 65  	  ;;
//! sh: 66  	esac
//! sh: 67  	return
//! sh: 68        fi
//! sh: 69        ;;
//! sh: 70
//! sh: 71        ([lr])
//! sh: 72        # One compulsory argument, two optional.
//! sh: 73        if [[ -z $PREFIX ]]; then
//! sh: 74  	_delimiters qualifier-$char
//! sh: 75  	return
//! sh: 76        else
//! sh: 77  	delim=$PREFIX[1]
//! sh: 78  	if ! _globqual_delims; then
//! sh: 79  	  # still completing argument
//! sh: 80  	  _message "padding width"
//! sh: 81  	  return
//! sh: 82  	fi
//! sh: 83  	# TBD if $PREFIX is empty can complete
//! sh: 84  	# either repeat delimiter or a new qualifier.
//! sh: 85  	# You might think it would just be easier
//! sh: 86  	# for the user to type the delimiter at
//! sh: 87  	# this stage, but users are astonishingly lazy.
//! sh: 88  	if [[ $delim = $PREFIX[1] ]]; then
//! sh: 89  	  # second argument
//! sh: 90  	  if ! _globqual_delims; then
//! sh: 91  	    _message "repeated padding"
//! sh: 92  	    return
//! sh: 93  	  fi
//! sh: 94  	  if [[ $delim = $PREFIX[1] ]]; then
//! sh: 95  	    if ! _globqual_delims; then
//! sh: 96  	      _message "one-off padding"
//! sh: 97  	      return
//! sh: 98  	    fi
//! sh: 99  	  fi
//! sh:100  	fi
//! sh:101        fi
//! sh:102        ;;
//! sh:103      esac
//! sh:104    done
//! sh:105
//! sh:106    if [[ -z $found_percent ]]; then
//! sh:107      flags=("%:expand prompt sequences")
//! sh:108    else
//! sh:109      flags=("%:expand prompts respecting options")
//! sh:110    fi
//! sh:111    case $q_last in
//! sh:112      (0)
//! sh:113      if (( n_q == 0 )); then
//! sh:114        flags+=("q:quote with backslashes")
//! sh:115      fi
//! sh:116      ;;
//! sh:117
//! sh:118      (1)
//! sh:119      flags+=(
//! sh:120        "q:quote with single quotes"
//! sh:121        "-:quote minimally for readability"
//! sh:122        "+:quote like q-, plus \$'...' for unprintable characters"
//! sh:123      )
//! sh:124      ;;
//! sh:125
//! sh:126      (2)
//! sh:127      flags+=("q:quote with double quotes")
//! sh:128      ;;
//! sh:129
//! sh:130      (3)
//! sh:131      flags+=("q:quote with \$'...'")
//! sh:132      ;;
//! sh:133    esac
//! sh:134    if (( !n_q )); then
//! sh:135      flags+=("Q:remove one level of quoting")
//! sh:136    fi
//! sh:137    if [[ -z $found_m ]]; then
//! sh:138      flags+=("m:count multibyte width in padding calculation")
//! sh:139    else
//! sh:140      flags+=("m:count number of character code points in padding calculation")
//! sh:141    fi
//! sh:142    flags+=(
//! sh:143      "#:interpret numeric expression as character code"
//! sh:144      "@:prevent double-quoted joining of arrays"
//! sh:145      "*:enable extended globs for pattern"
//! sh:146      "A:assign as an array parameter"
//! sh:147      "a:sort in array index order (with O to reverse)"
//! sh:148      "b:backslash quote pattern characters only"
//! sh:149      "c:count characters in an array (with \${(c)#...})"
//! sh:150      "C:capitalize words"
//! sh:151      "D:perform directory name abbreviation"
//! sh:152      "e:perform single-word shell expansions"
//! sh:153      "f:split the result on newlines"
//! sh:154      "F:join arrays with newlines"
//! sh:155      "g:process echo array sequences (needs options)"
//! sh:156      "i:sort case-insensitively"
//! sh:157      "k:substitute keys of associative arrays"
//! sh:158      "L:lower case all letters"
//! sh:159      "n:sort positive decimal integers numerically"
//! sh:160      "-:sort decimal integers numerically"
//! sh:161      "o:sort in ascending order (lexically if no other sort option)"
//! sh:162      "O:sort in descending order (lexically if no other sort option)"
//! sh:163      "P:use parameter value as name of parameter for redirected lookup"
//! sh:164      "t:substitute type of parameter"
//! sh:165      "u:substitute first occurrence of each unique word"
//! sh:166      "U:upper case all letters"
//! sh:167      "v:substitute values of associative arrays (with (k))"
//! sh:168      "V:visibility enhancements for special characters"
//! sh:169      "w:count words in array or string (with \${(w)#...})"
//! sh:170      "W:count words including empty words (with \${(W)#...})"
//! sh:171      "X:report parsing errors and eXit substitution"
//! sh:172      "z:split words as if zsh command line"
//! sh:173      "0:split words on null bytes"
//! sh:174      "p:handle print escapes or variables in parameter flag arguments"
//! sh:175      "~:treat strings in parameter flag arguments as patterns"
//! sh:176      "j:join arrays with specified string"
//! sh:177      "l:left-pad resulting words"
//! sh:178      "r:right-pad resulting words"
//! sh:179      "s:split words on specified string"
//! sh:180      "Z:split words as if zsh command line (with options)"
//! sh:181      # "_:extended flags, for future expansion"
//! sh:182      "S:match non-greedy in /, // or search substrings in % and # expressions"
//! sh:183      "I:search <argument>th match in #, %, / expressions"
//! sh:184      "B:include index of beginning of match in #, % expressions"
//! sh:185      "E:include index of one past end of match in #, % expressions"
//! sh:186      "M:include matched portion in #, % expressions"
//! sh:187      "N:include length of match in #, % expressions"
//! sh:188      "R:include rest (unmatched portion) in #, % expressions"
//! sh:189    )
//! sh:190    _describe -t flags "parameter flag" flags -Q -S ''
//! sh:191    return
//! sh:192  elif compset -P '*:([\|\*\^]|\^\^)'; then
//! sh:193    _arrays
//! sh:194    return
//! sh:195  elif compset -P '*:'; then
//! sh:196      flags=(
//! sh:197        '-:substitute alternate value if parameter is null'
//! sh:198        '+:substitute alternate value if parameter is non-null'
//! sh:199        '=:substitute and assign alternate value if parameter is null'
//! sh:200        '\:=:unconditionally assign value to parameter'
//! sh:201        '?:print error if parameter is null'
//! sh:202        '#:filter value matching pattern'
//! sh:203        '/:replace whole word matching pattern'
//! sh:204        '|:set difference'
//! sh:205        '*:set intersection'
//! sh:206        '^:zip arrays'
//! sh:207        '^^:zip arrays reusing values from shorter array'
//! sh:208      )
//! sh:209      _describe -t flags "operator" flags -Q -S ''
//! sh:210      _history_modifiers p
//! sh:211      return
//! sh:212  fi
//! sh:213
//! sh:214  _parameters -e
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
