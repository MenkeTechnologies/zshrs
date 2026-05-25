//! Port of `_globquals` from `Completion/Zsh/Type/_globquals`.
//!
//! Full upstream body (277 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  local state=qual expl char delim timespec default MATCH
//! sh:  4  integer MBEGIN MEND
//! sh:  5  local -a alts tdisp sdisp tmatch smatch
//! sh:  6  local -A specmap
//! sh:  7
//! sh:  8  while [[ -n $PREFIX ]]; do
//! sh:  9    char=$PREFIX[1]
//! sh: 10    compset -p 1
//! sh: 11    case $char in
//! sh: 12      ([-/F.@=p*rwxAIERWXsStUG^MTNDn,])
//! sh: 13      # no argument
//! sh: 14      ;;
//! sh: 15
//! sh: 16      (%)
//! sh: 17      # optional b, c
//! sh: 18      if [[ $PREFIX[1] = [bc] ]]; then
//! sh: 19        compset -p 1
//! sh: 20      fi
//! sh: 21      ;;
//! sh: 22
//! sh: 23      (f)
//! sh: 24      if ! compset -P "[-=+][0-7?]##"; then
//! sh: 25        if [[ -z $PREFIX ]]; then
//! sh: 26          _delimiters qualifier-f
//! sh: 27          return
//! sh: 28        elif ! _globqual_delims; then
//! sh: 29          # still completing mode spec
//! sh: 30          _message -e modes "mode spec"
//! sh: 31          return
//! sh: 32        fi
//! sh: 33      fi
//! sh: 34      ;;
//! sh: 35
//! sh: 36      (P)
//! sh: 37      # skip delimited prefix
//! sh: 38      if [[ -z $PREFIX ]]; then
//! sh: 39        _delimiters qualifier-P
//! sh: 40        return
//! sh: 41      elif ! _globqual_delims; then
//! sh: 42        # can't suggest anything here
//! sh: 43        _message -e prefix prefix
//! sh: 44        return
//! sh: 45      fi
//! sh: 46      ;;
//! sh: 47
//! sh: 48      (e)
//! sh: 49      # complete/skip delimited command line
//! sh: 50      if [[ -z $PREFIX ]]; then
//! sh: 51        _delimiters qualifier-e
//! sh: 52        return
//! sh: 53      elif ! _globqual_delims; then
//! sh: 54        # still completing command to eval
//! sh: 55        compset -q
//! sh: 56        _normal
//! sh: 57        return
//! sh: 58      fi
//! sh: 59      ;;
//! sh: 60
//! sh: 61      (+)
//! sh: 62      # complete/skip command name (no delimiters)
//! sh: 63      if [[ $PREFIX = [[:IDENT:]]# ]]; then
//! sh: 64        # either nothing there yet, or still on name
//! sh: 65        _command_names
//! sh: 66        return
//! sh: 67      fi
//! sh: 68      compset -P '[[:IDENT:]]##'
//! sh: 69      ;;
//! sh: 70
//! sh: 71      (d)
//! sh: 72      # complete/skip device
//! sh: 73      if ! compset -p '[[:digit:]]##'; then
//! sh: 74        _message -e device-ids "device ID"
//! sh: 75        return
//! sh: 76      fi
//! sh: 77      ;;
//! sh: 78
//! sh: 79      (l)
//! sh: 80      # complete/skip link count
//! sh: 81      if ! compset -P '([-+]|)[[:digit:]]##'; then
//! sh: 82        _message -e numbers "link count"
//! sh: 83        return
//! sh: 84      fi
//! sh: 85      ;;
//! sh: 86
//! sh: 87      (u)
//! sh: 88      # complete/skip UID or delimited user
//! sh: 89      if ! compset -P '[[:digit:]]##'; then
//! sh: 90        if [[ -z $PREFIX ]]; then
//! sh: 91          _delimiters qualifier-u
//! sh: 92          return
//! sh: 93        elif ! _globqual_delims; then
//! sh: 94          # still completing user
//! sh: 95          _users -S $delim
//! sh: 96          return
//! sh: 97        fi
//! sh: 98      fi
//! sh: 99      ;;
//! sh:100
//! sh:101      (g)
//! sh:102      # complete/skip GID or delimited group
//! sh:103      if ! compset -P '[[:digit:]]##'; then
//! sh:104        if [[ -z $PREFIX ]]; then
//! sh:105          _delimiters qualifier-g
//! sh:106          return
//! sh:107        elif ! _globqual_delims; then
//! sh:108          # still completing group
//! sh:109          _groups -S $delim
//! sh:110          return
//! sh:111        fi
//! sh:112      fi
//! sh:113      ;;
//! sh:114
//! sh:115      ([amc])
//! sh:116      if ! compset -P '([Mwhmsd]|)([-+]|)<->'; then
//! sh:117        # complete/skip relative time spec
//! sh:118        alts=()
//! sh:119        timespec=$PREFIX[1]
//! sh:120        if ! compset -P '[Mwhmsd]' && [[ -z $PREFIX ]]; then
//! sh:121  	tdisp=( seconds minutes hours days weeks Months )
//! sh:122  	tmatch=( s m h d w M )
//! sh:123  	if zstyle -T ":completion:${curcontext}:time-specifiers" verbose; then
//! sh:124  	  zstyle -s ":completion:${curcontext}:time-specifiers" list-separator sep || sep=--
//! sh:125            print -v tdisp -f "%s ${sep//(#m)[%\\]/$MATCH$MATCH} %s" ${tmatch:^^tdisp}
//! sh:126  	fi
//! sh:127  	alts+=( "time-specifiers:time specifier:compadd -E 0 -d tdisp -S '' -a tmatch" )
//! sh:128        fi
//! sh:129        if ! compset -P '[-+]' && [[ -z $PREFIX ]]; then
//! sh:130  	if zstyle -T ":completion:${curcontext}:senses" verbose; then
//! sh:131  	  zstyle -s ":completion:${curcontext}:senses" list-separator sep || sep=--
//! sh:132  	  default=" [default exactly]"
//! sh:133            sdisp=( "+ $sep before (older files)" "- $sep since (newer files)" )
//! sh:134  	  smatch=( + - )
//! sh:135  	else
//! sh:136  	  sdisp=( before exactly since )
//! sh:137  	  smatch=( + '' - )
//! sh:138  	fi
//! sh:139          alts+=( "senses:sense${default}:compadd -E 0 -d sdisp -S '' -a smatch" )
//! sh:140        fi
//! sh:141        specmap=( M months w weeks h hours m minutes s seconds '(|+|-|d)' days)
//! sh:142        alts+=('digits:digit ('${${specmap[(K)${timespec:-d}]}:-invalid time specifier}'):_dates -f ${${timespec/[-+]/d}:-d} -S ""' )
//! sh:143        _alternative $alts
//! sh:144        return
//! sh:145      fi
//! sh:146      ;;
//! sh:147
//! sh:148      (L)
//! sh:149      # complete/skip file size
//! sh:150      if ! compset -P '([kKmMgGtTpP]|)([-+]|)<->'; then
//! sh:151        # complete/skip size spec
//! sh:152        alts=()
//! sh:153        if ! compset -P '[kKmMgGtTpP]' && [[ -z $PREFIX ]]; then
//! sh:154          alts+=(
//! sh:155            "size-specifiers:size specifier:\
//! sh:156  ((k\:kb m\:mb g\:gb t\:tb p\:512-byte\ blocks))")
//! sh:157        fi
//! sh:158        if ! compset -P '[-+]' && [[ -z $PREFIX ]]; then
//! sh:159          alts+=("senses:sense:((-\:less\ than +\:more\ than))")
//! sh:160        fi
//! sh:161        alts+=('digits:digit: ')
//! sh:162        _alternative $alts
//! sh:163        return
//! sh:164      fi
//! sh:165      ;;
//! sh:166
//! sh:167      ([oO])
//! sh:168      # complete/skip sort spec
//! sh:169      if ! compset -p 1; then
//! sh:170        alts=(
//! sh:171          "n:lexical order of name"
//! sh:172          "L:size of file"
//! sh:173          "l:number of hard links"
//! sh:174          "a:last access time"
//! sh:175          "m:last modification time"
//! sh:176          "c:last inode change time"
//! sh:177          "d:directory depth"
//! sh:178          "N:no sorting"
//! sh:179          "e:execute code"
//! sh:180          "+:+ command name"
//! sh:181          )
//! sh:182        _describe -t sort-specifiers "sort specifier" alts -Q -S ''
//! sh:183        return
//! sh:184      elif [[ $IPREFIX[-1] = e ]]; then
//! sh:185        if [[ -z $PREFIX ]]; then
//! sh:186          _delimiters qualifier-oe
//! sh:187          return
//! sh:188        elif ! _globqual_delims; then
//! sh:189          compset -q
//! sh:190          _normal
//! sh:191          return
//! sh:192        fi
//! sh:193      elif [[ $IPREFIX[-1] = + ]]; then
//! sh:194        if [[ $PREFIX = [[:IDENT:]]# ]]; then
//! sh:195          # either nothing there yet, or still on name
//! sh:196          _command_names
//! sh:197          return
//! sh:198        fi
//! sh:199      fi
//! sh:200      ;;
//! sh:201
//! sh:202      (\[)
//! sh:203      # complete/skip range: check for closing bracket
//! sh:204      if ! compset -P "(-|)[[:digit:]]##(,(-|)[[:digit:]]##|)]"; then
//! sh:205        if compset -P "(-|)[[:digit:]]##,"; then
//! sh:206          _message "end of range"
//! sh:207        else
//! sh:208          _message "start of range"
//! sh:209        fi
//! sh:210        return
//! sh:211      fi
//! sh:212      ;;
//! sh:213
//! sh:214      (:)
//! sh:215      # complete modifiers and don't stop completing them
//! sh:216      _history_modifiers q
//! sh:217      return
//! sh:218      ;;
//! sh:219    esac
//! sh:220  done
//! sh:221
//! sh:222  case $state in
//! sh:223    (qual)
//! sh:224    local -a quals
//! sh:225    quals=(
//! sh:226      "/:directories"
//! sh:227      "F:non-empty directories"
//! sh:228      ".:plain files"
//! sh:229      "@:symbolic links"
//! sh:230      "=:sockets"
//! sh:231      "p:named pipes (FIFOs)"
//! sh:232      "*:executable plain files"
//! sh:233      "%:device files"
//! sh:234      "r:owner-readable"
//! sh:235      "w:owner-writeable"
//! sh:236      "x:owner-executable"
//! sh:237      "A:group-readable"
//! sh:238      "I:group-writeable"
//! sh:239      "E:group-executable"
//! sh:240      "R:world-readable"
//! sh:241      "W:world-writeable"
//! sh:242      "X:world-executable"
//! sh:243      "s:setuid"
//! sh:244      "S:setgid"
//! sh:245      "t:sticky bit set"
//! sh:246      "f:+ access rights"
//! sh:247      "e:execute code"
//! sh:248      "+:+ command name"
//! sh:249      "d:+ device"
//! sh:250      "l:+ link count"
//! sh:251      "U:owned by EUID"
//! sh:252      "G:owned by EGID"
//! sh:253      "u:+ owning user"
//! sh:254      "g:+ owning group"
//! sh:255      "a:+ access time"
//! sh:256      "m:+ modification time"
//! sh:257      "c:+ inode change time"
//! sh:258      "L:+ size"
//! sh:259      "^:negate qualifiers"
//! sh:260      "-:follow symlinks toggle"
//! sh:261      "M:mark directories"
//! sh:262      "T:mark types"
//! sh:263      "N:use NULL_GLOB"
//! sh:264      "D:glob dots"
//! sh:265      "n:numeric glob sort"
//! sh:266      "o:+ sort order, up"
//! sh:267      "O:+ sort order, down"
//! sh:268      "P:prepend word"
//! sh:269      "Y:+ at most ARG matches"
//! sh:270      "[:+ range of files"
//! sh:271      ",:logical OR"
//! sh:272      "):end of qualifiers"
//! sh:273      "\::modifier"
//! sh:274      )
//! sh:275    _describe -t globquals "glob qualifier" quals -Q -S ''
//! sh:276    ;;
//! sh:277  esac
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
