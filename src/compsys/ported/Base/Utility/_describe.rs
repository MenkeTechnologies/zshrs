//! Port of `_describe` from `Completion/Base/Utility/_describe`.
//!
//! Full upstream body (140 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # ### Note: Calling this function twice during one completion operation, such
//! sh:  4  # ### that in each call there exists a pair of items having the same description
//! sh:  5  # ### as each other, and the two calls specify the same $_type, currently leads
//! sh:  6  # ### to garbled output; see workers/35229 (May 2015) and its thread (which also
//! sh:  7  # ### discusses at least two other issues, that may or may not be related to
//! sh:  8  # ### this one).
//! sh:  9
//! sh: 10  # This can be used to add options or values with descriptions as matches.
//! sh: 11
//! sh: 12  local _opt _expl _tmpm _tmpd _mlen _noprefix
//! sh: 13  local _type=values _descr _ret=1 _showd _nm _hide _args _grp _sep
//! sh: 14  local csl="$compstate[list]" csl2
//! sh: 15  local _oargv _argv _new _strs _mats _opts _i _try=0
//! sh: 16  local OPTIND OPTARG
//! sh: 17  local -a _jvx12
//! sh: 18
//! sh: 19  # Get the option.
//! sh: 20
//! sh: 21  while getopts "oOt:12JVx" _opt; do
//! sh: 22    case $_opt in
//! sh: 23      (o)
//! sh: 24        _type=options;;
//! sh: 25      (O)
//! sh: 26        _type=options
//! sh: 27        _noprefix=1
//! sh: 28        ;;
//! sh: 29      (t)
//! sh: 30        _type="$OPTARG"
//! sh: 31        ;;
//! sh: 32      (1|2|J|V|x)
//! sh: 33        _jvx12+=(-$_opt)
//! sh: 34    esac
//! sh: 35  done
//! sh: 36  shift $(( OPTIND - 1 ))
//! sh: 37  unset _opt
//! sh: 38
//! sh: 39  [[ "$_type$_noprefix" = options && ! -prefix [-+]* ]] && \
//! sh: 40      zstyle -T ":completion:${curcontext}:options" prefix-needed &&
//! sh: 41          return 1
//! sh: 42
//! sh: 43  # Do the tests. `showd' is set if the descriptions should be shown.
//! sh: 44
//! sh: 45  zstyle -T ":completion:${curcontext}:$_type" verbose && _showd=yes
//! sh: 46
//! sh: 47  zstyle -s ":completion:${curcontext}:$_type" list-separator _sep || _sep=--
//! sh: 48  zstyle -s ":completion:${curcontext}:$_type" max-matches-width _mlen ||
//! sh: 49      _mlen=$((COLUMNS/2))
//! sh: 50
//! sh: 51  _descr="$1"
//! sh: 52  shift
//! sh: 53
//! sh: 54  if [[ -n "$_showd" ]] &&
//! sh: 55     zstyle -T ":completion:${curcontext}:$_type" list-grouped; then
//! sh: 56    _oargv=( "$@" )
//! sh: 57    _grp=(-g)
//! sh: 58  else
//! sh: 59    _grp=()
//! sh: 60  fi
//! sh: 61
//! sh: 62  [[ "$_type" = options ]] &&
//! sh: 63      zstyle -t ":completion:${curcontext}:options" prefix-hidden &&
//! sh: 64          _hide="${(M)PREFIX##(--|[-+])}"
//! sh: 65
//! sh: 66  _tags "$_type"
//! sh: 67  while _tags; do
//! sh: 68    while _next_label $_jvx12 "$_type" _expl "$_descr"; do
//! sh: 69
//! sh: 70      if (( $#_grp )); then
//! sh: 71
//! sh: 72        set -- "$_oargv[@]"
//! sh: 73        _argv=( "$_oargv[@]" )
//! sh: 74        _i=1
//! sh: 75        (( _try++ ))
//! sh: 76        while (( $# )); do
//! sh: 77
//! sh: 78          _strs="_a_$_try$_i"
//! sh: 79          if [[ "$1" = \(*\) ]]; then
//! sh: 80            eval local "_a_$_try$_i;_a_$_try$_i"'='$1
//! sh: 81          else
//! sh: 82            eval local "_a_$_try$_i;_a_$_try$_i"'=( "${'$1'[@]}" )'
//! sh: 83          fi
//! sh: 84          _argv[_i]="_a_$_try$_i"
//! sh: 85          shift
//! sh: 86          (( _i++ ))
//! sh: 87
//! sh: 88          if [[ "$1" = (|-*) ]]; then
//! sh: 89            _mats=
//! sh: 90          else
//! sh: 91            _mats="_a_$_try$_i"
//! sh: 92            if [[ "$1" = \(*\) ]]; then
//! sh: 93              eval local "_a_$_try$_i;_a_$_try$_i"'='$1
//! sh: 94            else
//! sh: 95              eval local "_a_$_try$_i;_a_$_try$_i"'=( "${'$1'[@]}" )'
//! sh: 96            fi
//! sh: 97            _argv[_i]="_a_$_try$_i"
//! sh: 98            shift
//! sh: 99            (( _i++ ))
//! sh:100          fi
//! sh:101
//! sh:102          _opts=( "${(@)argv[1,(i)--]:#--}" )
//! sh:103          shift "$#_opts"
//! sh:104          (( _i += $#_opts ))
//! sh:105          if [[ $1 == -- ]]; then
//! sh:106            shift
//! sh:107            (( _i++ ))
//! sh:108          fi
//! sh:109
//! sh:110          if [[ -n $_mats ]]; then
//! sh:111            compadd "$_opts[@]" -2 -o nosort "${_expl[@]}" -D $_strs -O $_mats - \
//! sh:112                    "${(@)${(@M)${(@P)_mats}##([^:\\]|\\?)##}//\\(#b)(?)/$match[1]}"
//! sh:113          else
//! sh:114            compadd "$_opts[@]" -2 -o nosort "${_expl[@]}" -D $_strs - \
//! sh:115                    "${(@)${(@M)${(@P)_strs}##([^:\\]|\\?)##}//\\(#b)(?)/$match[1]}"
//! sh:116          fi
//! sh:117        done
//! sh:118        set - "$_argv[@]"
//! sh:119      fi
//! sh:120
//! sh:121      if [[ -n "$_showd" ]]; then
//! sh:122        compdescribe -I "$_hide" "$_mlen" "$_sep " _expl "$_grp[@]" "$@"
//! sh:123      else
//! sh:124        compdescribe -i "$_hide" "$_mlen" "$@"
//! sh:125      fi
//! sh:126
//! sh:127      compstate[list]="$csl"
//! sh:128
//! sh:129      while compdescribe -g csl2 _args _tmpm _tmpd; do
//! sh:130
//! sh:131        compstate[list]="$csl $csl2"
//! sh:132        [[ -n "$csl2" ]] && compstate[list]="${compstate[list]:s/rows//}"
//! sh:133
//! sh:134        compadd "$_args[@]" -d _tmpd -a _tmpm && _ret=0
//! sh:135      done
//! sh:136    done
//! sh:137    (( _ret )) || return 0
//! sh:138  done
//! sh:139
//! sh:140  return 1
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
