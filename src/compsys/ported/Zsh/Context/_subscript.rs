//! Port of `_subscript` from `Completion/Zsh/Context/_subscript`.
//!
//! Full upstream body (136 lines verbatim):
//! ```text
//! sh:  1  #compdef -subscript-
//! sh:  2
//! sh:  3  local expl ind osuf flags sep
//! sh:  4
//! sh:  5  [[ $ISUFFIX = *\]* ]] || osuf=\]
//! sh:  6
//! sh:  7  if [[ "$1" = -q ]]; then
//! sh:  8    compquote osuf
//! sh:  9    osuf+=' '
//! sh: 10    shift
//! sh: 11  fi
//! sh: 12
//! sh: 13  compset -P '\(([^\(\)]|\(*\))##\)' # remove subscript flags
//! sh: 14
//! sh: 15  # Look for a dynamic name expansion.  Completion only gives us
//! sh: 16  # the stuff inside the square brackets; we need to find out what's
//! sh: 17  # outside.  We ought to check for quoting, really, but given we've
//! sh: 18  # got to the subscript code " ~[" is pretty likely to be a dynamic
//! sh: 19  # name expansion.  Also expand in anything that looks like an assignment
//! sh: 20  # or colon list.
//! sh: 21  integer pos=$((CURSOR+1))
//! sh: 22  while [[ pos -gt 1 && $BUFFER[pos-1] != '[' ]]; do (( pos-- )); done
//! sh: 23  if [[ $BUFFER[1,pos-1] = (|*[[:space:]:=]##)\~\[ ]]; then
//! sh: 24    _dynamic_directory_name
//! sh: 25  elif [[ "$PREFIX" = :* ]]; then
//! sh: 26    _wanted characters expl 'character class' \
//! sh: 27        compadd -p: -S ':]' alnum alpha ascii blank cntrl digit graph \
//! sh: 28        lower print punct space upper xdigit IFS IDENT IFSSPACE WORD
//! sh: 29  elif compset -P '\('; then
//! sh: 30    local match
//! sh: 31    compset -S '\)*'
//! sh: 32
//! sh: 33    if [[ $PREFIX = (#b)*([bns])(?|)(*) ]]; then
//! sh: 34      local f=$match[1] d=$match[2] e=$match[2] v=$match[3]
//! sh: 35      [[ $f = s && ${(Pt)${compstate[parameter]}} != scalar* ]] && return 1
//! sh: 36      if [[ -z $d ]]; then
//! sh: 37        _message -e delimiters 'delimiter'
//! sh: 38        return
//! sh: 39      else
//! sh: 40        case $d in
//! sh: 41        (\() e=\);;
//! sh: 42        (\[) e=\];;
//! sh: 43        (\{) e=\};;
//! sh: 44        esac
//! sh: 45        if [[ $v != *$e* ]]; then
//! sh: 46  	case $f in
//! sh: 47  	(s) _message 'separator string';;
//! sh: 48  	(b|n) [[ $v = <-># ]] && _message 'number' || return 1;;
//! sh: 49  	esac
//! sh: 50  	[[ -n $v && $SUFFIX$ISUFFIX != *$e* ]] && _message 'delimiter'
//! sh: 51  	return 0
//! sh: 52        fi
//! sh: 53      fi
//! sh: 54    fi
//! sh: 55
//! sh: 56    case ${(Pt)${compstate[parameter]}} in
//! sh: 57      assoc*) flags=(
//! sh: 58        '(R k K i I)r[any one value matched by subscript as pattern]'
//! sh: 59        '(r k K i I)R[all values matched by subscript as pattern]'
//! sh: 60        '(r R K i I)k[any one value where subscript matched by key as pattern]'
//! sh: 61        '(r R k i I)K[all values where subscript matched by key as pattern]'
//! sh: 62        '(r R k K I)i[any one key matched by subscript as pattern]'
//! sh: 63        '(r R k K i)I[all keys matched by subscript as pattern]'
//! sh: 64        'e[interpret * or @ as a single key]'
//! sh: 65      );;
//! sh: 66      (|scalar*)) flags=(
//! sh: 67        'f[make subscripting work on lines of scalar]'
//! sh: 68        'w[make subscripting work on words of scalar]'
//! sh: 69        's[specify word separator]'
//! sh: 70        'p[recognise escape sequences in subsequent s flag]'
//! sh: 71      );&
//! sh: 72      array*) flags=($flags
//! sh: 73        'e[interpret * or @ as a single key and use plain string matching]'
//! sh: 74        'n[Nth lowest/highest index with i/I/r/R flag]'
//! sh: 75        'b[begin with specified element]'
//! sh: 76        '(r R k K i)I[highest index of value matched by subscript]'
//! sh: 77        '(r R k K I)i[lowest index of value matched by subscript]'
//! sh: 78        '(r k K i I)R[value matched by subscript at highest index]'
//! sh: 79        '(R k K i I)r[value matched by subscript at lowest index]'
//! sh: 80      );;
//! sh: 81    esac
//! sh: 82
//! sh: 83    _values -s '' 'subscript flag' $flags
//! sh: 84  elif [[ ${(Pt)${compstate[parameter]}} = assoc* ]]; then
//! sh: 85    local suf MATCH MBEGIN MEND
//! sh: 86    local -a keys
//! sh: 87    keys=("${(@)${(@k)${(P)compstate[parameter]}}//(#m)[\$\\\[\]\(\)\{\}]/\\$MATCH}")
//! sh: 88    keys=("${(@)keys//#%(#m)[*@]/(e)$MATCH}")
//! sh: 89    [[ "$RBUFFER" != (|\\)\]* ]] && suf="$osuf"
//! sh: 90
//! sh: 91    _wanted association-keys expl 'association key' \
//! sh: 92        compadd -Q -S "$suf" -a keys
//! sh: 93  elif [[ ${(Pt)${compstate[parameter]}} = array* ]]; then
//! sh: 94    local list i j ret=1 disp
//! sh: 95
//! sh: 96    _tags indexes parameters
//! sh: 97
//! sh: 98    while _tags; do
//! sh: 99      if _requested indexes; then
//! sh:100        ind=( {1..${#${(P)${compstate[parameter]}}}} )
//! sh:101        if [[ ${ind[-1]} -eq 0 ]]; then
//! sh:102          ind=()
//! sh:103        fi
//! sh:104        if zstyle -T ":completion:${curcontext}:indexes" verbose; then
//! sh:105          list=()
//! sh:106          for i in "$ind[@]"; do
//! sh:107            if [[ "$i" = ${PREFIX}*${SUFFIX} ]]; then
//! sh:108                list+=( "${i}:$(print -D -- ${(P)${compstate[parameter]}[$i]})" )
//! sh:109  	  else
//! sh:110  	      list+=( '' )
//! sh:111  	  fi
//! sh:112          done
//! sh:113          zstyle -s ":completion:${curcontext}:indexes" list-separator sep || sep=--
//! sh:114          zformat -a list " $sep " "$list[@]"
//! sh:115  	disp=( -d list)
//! sh:116        else
//! sh:117          disp=()
//! sh:118        fi
//! sh:119
//! sh:120        if [[ "$RBUFFER" = (|\\)\]* ]]; then
//! sh:121          _all_labels -V indexes expl 'array index' \
//! sh:122              compadd -S '' "$disp[@]" -a ind && ret=0
//! sh:123        else
//! sh:124          _all_labels -V indexes expl 'array index' \
//! sh:125              compadd -S "$osuf" "$disp[@]" -a ind && ret=0
//! sh:126        fi
//! sh:127      fi
//! sh:128      _requested parameters && _parameters && ret=0
//! sh:129
//! sh:130      (( ret )) || return 0
//! sh:131    done
//! sh:132
//! sh:133    return 1
//! sh:134  else
//! sh:135    _dispatch -math- -math-
//! sh:136  fi
//! ```
//!
//! Strict Rust port: caller injects the parameter table (so we
//! can resolve `$compstate[parameter]` to its type). For assoc
//! arrays, emit the keys. For regular arrays, emit integer
//! indices `1..N`. For scalars, emit `1`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
