//! Port of `_path_commands` from `Completion/Unix/Type/_path_commands`.
//!
//! Full upstream body (125 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  (( $+functions[_path_commands_caching_policy] )) ||
//! sh:  4  _path_commands_caching_policy() {
//! sh:  5
//! sh:  6  local file
//! sh:  7  local -a oldp dbfiles
//! sh:  8
//! sh:  9  # rebuild if cache is more than a week old
//! sh: 10  oldp=( "$1"(Nmw+1) )
//! sh: 11  (( $#oldp )) && return 0
//! sh: 12
//! sh: 13  dbfiles=(/usr/share/man/index.(bt|db|dir|pag)(N) \
//! sh: 14    /usr/man/index.(bt|db|dir|pag)(N) \
//! sh: 15    /var/cache/man/index.(bt|db|dir|pag)(N) \
//! sh: 16    /var/catman/index.(bt|db|dir|pag)(N) \
//! sh: 17    /usr/share/man/*/whatis(N))
//! sh: 18
//! sh: 19  for file in $dbfiles; do
//! sh: 20    [[ $file -nt $1 ]] && return 0
//! sh: 21  done
//! sh: 22
//! sh: 23  return 1
//! sh: 24  }
//! sh: 25
//! sh: 26  _call_whatis() {
//! sh: 27    local sec impl variant sections=( 1 6 8 )
//! sh: 28    case "$OSTYPE" in
//! sh: 29      (#i)dragonfly|(free|open)bsd*) impl=mandoc ;;
//! sh: 30      netbsd*) impl=apropos ;;
//! sh: 31      linux-gnu*)
//! sh: 32        sections=( 1 8 )
//! sh: 33        # The same test as for man so has a good chance of being cached
//! sh: 34        _pick_variant -c man -r variant \
//! sh: 35          freebsd='-S mansect' \
//! sh: 36          openbsd='-S subsection' \
//! sh: 37          $OSTYPE \
//! sh: 38          ---
//! sh: 39        [[ $variant = $OSTYPE ]] && impl=man-db || impl=mandoc
//! sh: 40      ;;
//! sh: 41    esac
//! sh: 42    case $impl in
//! sh: 43      mandoc)
//! sh: 44        for sec in $sections; do
//! sh: 45          whatis -s $sec .\*
//! sh: 46        done
//! sh: 47      ;;
//! sh: 48      man-db)
//! sh: 49        whatis -s ${(j.,.)sections} -r .\*
//! sh: 50      ;;
//! sh: 51      apropos)
//! sh: 52        apropos -l ''|grep "([${(j..)sections}])"
//! sh: 53      ;;
//! sh: 54    esac
//! sh: 55  }
//! sh: 56
//! sh: 57  _path_commands() {
//! sh: 58  local need_desc expl ret=1
//! sh: 59
//! sh: 60  if zstyle -t ":completion:${curcontext}:commands" extra-verbose; then
//! sh: 61    local update_policy first
//! sh: 62    if [[ $+_command_descriptions -eq 0 ]]; then
//! sh: 63      first=yes
//! sh: 64      typeset -A -g _command_descriptions
//! sh: 65    fi
//! sh: 66    zstyle -s ":completion:${curcontext}:" cache-policy update_policy
//! sh: 67    [[ -z "$update_policy" ]] && zstyle ":completion:${curcontext}:" \
//! sh: 68      cache-policy _path_commands_caching_policy
//! sh: 69    if ( [[ -n $first ]] || _cache_invalid command-descriptions ) && \
//! sh: 70      ! _retrieve_cache command-descriptions; then
//! sh: 71      local line
//! sh: 72      for line in "${(f)$(_call_program command-descriptions _call_whatis)}"; do
//! sh: 73        [[ -n ${line:#(#b)([^ ]#) #\([^ ]#\)( #\[[^ ]#\]|)[ -]#(*)} ]] && continue;
//! sh: 74        [[ -z $match[1] || -z $match[3] || -z ${${match[1]}:#*:*} ]] && continue;
//! sh: 75        _command_descriptions[$match[1]]=$match[3]
//! sh: 76      done
//! sh: 77      _store_cache command-descriptions _command_descriptions
//! sh: 78    fi
//! sh: 79
//! sh: 80    (( $#_command_descriptions )) && need_desc=yes
//! sh: 81  fi
//! sh: 82
//! sh: 83  if [[ -n $need_desc ]]; then
//! sh: 84    typeset -a dcmds descs cmds matches
//! sh: 85    local desc cmd sep
//! sh: 86    compadd "$@" -O matches -k commands
//! sh: 87    for cmd in $matches; do
//! sh: 88      desc=$_command_descriptions[$cmd]
//! sh: 89      if [[ -z $desc ]]; then
//! sh: 90        cmds+=$cmd
//! sh: 91      else
//! sh: 92        dcmds+=$cmd
//! sh: 93        descs+="$cmd:$desc"
//! sh: 94      fi
//! sh: 95    done
//! sh: 96    zstyle -s ":completion:${curcontext}:" list-separator sep || sep=--
//! sh: 97    zformat -a descs " $sep " $descs
//! sh: 98    descs=("${(@r:COLUMNS-1:)descs}")
//! sh: 99    _wanted commands expl 'external command' \
//! sh:100      compadd "$@" -ld descs -a dcmds && ret=0
//! sh:101    _wanted commands expl 'external command' compadd "$@" -a cmds && ret=0
//! sh:102  else
//! sh:103    _wanted commands expl 'external command' compadd "$@" -k commands && ret=0
//! sh:104  fi
//! sh:105  # TODO: this is called from '_command_names -e' which is typically used in
//! sh:106  # contexts (such as _env) that don't accept directory names.  Should this
//! sh:107  # 'if' block move up to the "_command_names -" branch of _command_names?
//! sh:108  if [[ -o path_dirs ]]; then
//! sh:109    local -a path_dirs
//! sh:110
//! sh:111    if [[ $PREFIX$SUFFIX = */* ]]; then
//! sh:112      path_dirs=( ${path:#.} )
//! sh:113      # Find command from path, not hashed
//! sh:114      _wanted commands expl 'external command' _path_files -W path_dirs -g '*(-*)' && ret=0
//! sh:115    else
//! sh:116      path_dirs=(${^path}/*(/N:t))
//! sh:117      (( ${#path_dirs} )) &&
//! sh:118          _wanted path-dirs expl 'directory in path' compadd "$@" -S / -a path_dirs && ret=0
//! sh:119    fi
//! sh:120  fi
//! sh:121
//! sh:122  return ret
//! sh:123  }
//! sh:124
//! sh:125  _path_commands "$@"
//! ```
//!
//! `compadd -k commands` is shorthand for "emit every external
//! executable found in $path" — exactly what [`_command_names`] in
//! externals-only mode does.
//!
//! Strict Rust port: wraps the emission with [`_wanted`] on the
//! `commands` tag with `'external command'` description (matching
//! upstream verbatim), then dispatches to [`_command_names`] with
//! `externals_only=true`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
