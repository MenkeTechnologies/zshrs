//! Port of `_arg_compile` from `Completion/Base/Utility/_arg_compile`.
//!
//! Full upstream body (199 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # A simple compiler for _arguments descriptions.  The first argument of
//! sh:  4  # _arg_compile is the name of an array parameter in which the parse is
//! sh:  5  # returned.  The remaining arguments form a series of `phrases'.  Each
//! sh:  6  # `phrase' begins with one of the keywords "argument", "option", or "help"
//! sh:  7  # and consists of a series of keywords and/or values.  The syntax is as
//! sh:  8  # free-form as possible, but "argument" phrases generally must appear in
//! sh:  9  # the same relative position as the corresponding argument on the command
//! sh: 10  # line to be completed, and there are some restrictions on ordering of
//! sh: 11  # keywords and values within each phrase.
//! sh: 12  #
//! sh: 13  # Anything appearing before the first phrase or after the last is passed
//! sh: 14  # through verbatim.  (See TODO.)  If more detailed mixing of compiled and
//! sh: 15  # uncompiled fragments is necessary, use two or more calls, either with
//! sh: 16  # different array names or by passing the output of each previous call
//! sh: 17  # through the next.
//! sh: 18  #
//! sh: 19  # In the documentation below, brackets [ ] indicate optional elements and
//! sh: 20  # braces { } indicate elements that may be repeated zero or more times.
//! sh: 21  # Except as noted, bracketed or braced elements may appear in any order
//! sh: 22  # relative to each other, but tokens within each element are ordered.
//! sh: 23  #
//! sh: 24  #   argument [POS] [means MSG] [action ACT]
//! sh: 25  #
//! sh: 26  #     POS may be an integer N for the Nth argument or "*" for all, and
//! sh: 27  #      must appear first if it appears at all.
//! sh: 28  #     MSG is a string to be displayed above the matches in a listing.
//! sh: 29  #     ACT is (currently) as described in the compsys manual.
//! sh: 30  #
//! sh: 31  #   option OPT [follow HOW] [explain STR] {unless XOR} \
//! sh: 32  #    {[means MSG] [action ACT]} [through PAT [means MSG] [action ACT]]
//! sh: 33  #
//! sh: 34  #     OPT is the option, prefixed with "*" if it may appear more than once.
//! sh: 35  #     HOW refers to a following argument, and may be one of:
//! sh: 36  #       "close"   must appear in the same word (synonyms "join" or "-")
//! sh: 37  #       "next"    the argument must appear in the next word (aka "split")
//! sh: 38  #       "loose"   the argument may appear in the same or the next word ("+")
//! sh: 39  #       "assign"  as loose, but must follow an "=" in the same word ("=")
//! sh: 40  #     HOW should be suffixed with a colon if the following argument is
//! sh: 41  #      _not_ required to appear.
//! sh: 42  #     STR is to be displayed based on style `description'
//! sh: 43  #     XOR is another option in combination with which OPT may not appear.
//! sh: 44  #      It may be ":" to disable non-option completions when OPT is present.
//! sh: 45  #     MSG is a string to be displayed above the matches in a listing.
//! sh: 46  #     ACT is (currently) as described in the compsys manual.
//! sh: 47  #     PAT is either "*" for "all remaining words on the line" or a pattern
//! sh: 48  #      that, if matched, marks the end of the arguments of this option.
//! sh: 49  #      The "through PAT ..." description must be the last.
//! sh: 50  #     PAT may be suffixed with one colon to narrow the $words array to
//! sh: 51  #      the remainder of the command line, or with two colons to narrow
//! sh: 52  #      to the words before (not including) the next that matches PAT.
//! sh: 53  #
//! sh: 54  #   help PAT [means MSG] action ACT
//! sh: 55  #
//! sh: 56  #     ACT is applied to any option output by --help that matches PAT.
//! sh: 57  #      Do not use "help" with commands that do not support --help.
//! sh: 58  #     PAT may be suffixed with a colon if the following argument is
//! sh: 59  #      _not_ required to appear (this is usually inferred from --help).
//! sh: 60  #     MSG is a string to be displayed above the matches in a listing.
//! sh: 61
//! sh: 62  # EXAMPLE:
//! sh: 63  # This is from _gprof in the standard distribution.  Note that because of
//! sh: 64  # the brace expansion trick used in the "function name" case, no attempt
//! sh: 65  # is made to use `phrase' form; that part gets passed through unchanged.
//! sh: 66  # It could simply be moved to the _arguments call ahead of "$args[@]".
//! sh: 67  #
//! sh: 68  # _arg_compile args -s -{a,b,c,D,h,i,l,L,s,T,v,w,x,y,z} \
//! sh: 69  #              -{A,C,e,E,f,F,J,n,N,O,p,P,q,Q,Z}:'function name:->funcs' \
//! sh: 70  #              option -I means directory action _dir_list \
//! sh: 71  #              option -d follow close means "debug level" \
//! sh: 72  #              option -k means "function names" action '->pair' \
//! sh: 73  #              option -m means "minimum execution count" \
//! sh: 74  #              argument means executable action '_files -g \*\(-\*\)' \
//! sh: 75  #              argument means "profile file" action '_files -g gmon.\*' \
//! sh: 76  #              help '*=name*' means "function name" action '->funcs' \
//! sh: 77  #              help '*=dirs*' means "directory" action _dir_list
//! sh: 78  # _arguments "$args[@]"
//! sh: 79
//! sh: 80  # TODO:
//! sh: 81  # Verbose forms of various actions, e.g. (but not exactly)
//! sh: 82  #   "state foo"                  becomes "->foo"
//! sh: 83  #   "completion X explain Y ..." becomes "((X\:Y ...))"
//! sh: 84  #   etc.
//! sh: 85  # Represent leading "*" in OPT some other way.
//! sh: 86  # Represent trailing colons in HOW and PAT some other way.
//! sh: 87  # Stricter syntax checking on HOW, sanity checks on XOR.
//! sh: 88  # Something less obscure than "unless :" would be nice.
//! sh: 89  # Warning or other syntax check for stuff after the last phrase.
//! sh: 90
//! sh: 91  emulate -L zsh
//! sh: 92  local -h argspec dspec helpspec prelude xor
//! sh: 93  local -h -A amap dmap safe
//! sh: 94
//! sh: 95  [[ -n "$1" ]] || return 1
//! sh: 96  [[ ${(tP)${1}} = *-local ]] && { print -R NAME CONFLICT: $1 1>&2; return 1 }
//! sh: 97  safe[reply]="$1"; shift
//! sh: 98
//! sh: 99  # First consume and save anything before the argument phrases
//! sh:100
//! sh:101  helpspec=()
//! sh:102  prelude=()
//! sh:103
//! sh:104  while (($#))
//! sh:105  do
//! sh:106    case $1 in
//! sh:107    (argument|help|option) break;;
//! sh:108    (*) prelude=("$prelude[@]" "$1"); shift;;
//! sh:109    esac
//! sh:110  done
//! sh:111
//! sh:112  # Consume all the argument phrases and build the argspec array
//! sh:113
//! sh:114  while (($#))
//! sh:115  do
//! sh:116    amap=()
//! sh:117    dspec=()
//! sh:118    case $1 in
//! sh:119
//! sh:120    # argument [POS] [means MSG] [action ACT]
//! sh:121    (argument)
//! sh:122      shift
//! sh:123      while (($#))
//! sh:124      do
//! sh:125        case $1 in
//! sh:126        (<1->|\*) amap[position]="$1"; shift;;
//! sh:127        (means|action) amap[$1]="$2"; shift 2;;
//! sh:128        (argument|option|help) break;;
//! sh:129        (*) print -R SYNTAX ERROR at "$@" 1>&2; return 1;;
//! sh:130        esac
//! sh:131      done
//! sh:132      if (( $#amap ))
//! sh:133      then
//! sh:134        argspec=("$argspec[@]" "${amap[position]}:${amap[means]}:${amap[action]}")
//! sh:135      fi;;
//! sh:136
//! sh:137    # option OPT [follow HOW] [explain STR] {unless XOR} \
//! sh:138    #  {[through PAT] [means MSG] [action ACT]}
//! sh:139    (option)
//! sh:140      amap[option]="$2"; shift 2
//! sh:141      dmap=()
//! sh:142      xor=()
//! sh:143      while (( $# ))
//! sh:144      do
//! sh:145        (( ${+amap[$1]} || ${+dmap[through]} )) && break;
//! sh:146        case $1 in
//! sh:147        (follow)
//! sh:148  	amap[follow]="${2:s/join/-/:s/close/-/:s/next//:s/split//:s/loose/+/:s/assign/=/:s/none//}"
//! sh:149  	shift 2;;
//! sh:150        (explain) amap[explain]="[$2]" ; shift 2;;
//! sh:151        (unless) xor=("$xor[@]" "${(@)=2}"); shift 2;;
//! sh:152        (through|means|action)
//! sh:153  	while (( $# ))
//! sh:154  	do
//! sh:155  	  (( ${+dmap[$1]} )) && break 2
//! sh:156  	  case $1 in
//! sh:157  	  (through|means|action) dmap[$1]=":${2}"; shift 2;;
//! sh:158  	  (argument|option|help|follow|explain|unless) break;;
//! sh:159  	  (*) print -R SYNTAX ERROR at "$@" 1>&2; return 1;;
//! sh:160  	  esac
//! sh:161  	done;;
//! sh:162        (argument|option|help) break;;
//! sh:163        (*) print -R SYNTAX ERROR at "$@" 1>&2; return 1;;
//! sh:164        esac
//! sh:165        if (( $#dmap ))
//! sh:166        then
//! sh:167  	dspec=("$dspec[@]" "${dmap[through]}${dmap[means]:-:}${dmap[action]:-:}")
//! sh:168        fi
//! sh:169      done
//! sh:170      if (( $#amap ))
//! sh:171      then
//! sh:172        argspec=("$argspec[@]" "${xor:+($xor)}${amap[option]}${amap[follow]}${amap[explain]}${dspec}")
//! sh:173      fi;;
//! sh:174
//! sh:175    # help PAT [means MSG] action ACT
//! sh:176    (help)
//! sh:177      amap[pattern]="$2"; shift 2
//! sh:178      while (($#))
//! sh:179      do
//! sh:180        (( ${+amap[$1]} )) && break;
//! sh:181        case $1 in
//! sh:182        (means|action) amap[$1]="$2"; shift 2;;
//! sh:183        (argument|option|help) break;;
//! sh:184        (*) print -R SYNTAX ERROR at "$@" 1>&2; return 1;;
//! sh:185        esac
//! sh:186      done
//! sh:187      if (( $#amap ))
//! sh:188      then
//! sh:189        helpspec=("$helpspec[@]" "${amap[pattern]}:${amap[means]}:${amap[action]}")
//! sh:190      fi;;
//! sh:191    (*) break;;
//! sh:192    esac
//! sh:193  done
//! sh:194
//! sh:195  eval $safe[reply]'=( "${prelude[@]}" "${argspec[@]}" ${helpspec:+"-- ${helpspec[@]}"} "$@" )'
//! sh:196
//! sh:197  # print -R _arguments "${prelude[@]:q}" "${argspec[@]:q}" ${helpspec:+"-- ${helpspec[@]:q}"} "$@:q"
//! sh:198
//! sh:199  return 0
//! ```
//!
//! Faithful Rust port: every shape above is recognised and surfaced
//! as a typed field on `CompiledArgSpec`. The previous one-shape
//! parser (just `name:desc:action`) was a stub; this is the full
//! grammar.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
