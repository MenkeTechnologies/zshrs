setopt bsd_echo no_banghist no_multios no_equals interactive_comments no_shwordsplit no_ksharrays
__fish_echo() {
  local nl=1 sep=' ' esc= a i c out
  while (( $# )); do
    a=$1
    if [[ $a == -- ]]; then shift; break; fi
    [[ $a =~ '^-[nseE]+$' ]] || break
    for (( i = 2; i <= ${#a}; i++ )); do
      c=${a[i]}
      case $c in n) nl= ;; s) sep= ;; e) esc=1 ;; E) esc= ;; esac
    done
    shift
  done
  out=${(pj.$sep.)@}
  # fish reads `\0` and up to three octal digits; zsh reads `\0` with up to three digits differently
  if [[ -n $esc ]]; then
    setopt localoptions extendedglob
    # `\c` ends the output, newline included; an escape fish does not know is kept as written
    if [[ $out == *'\c'* ]]; then out=${out%%\\c*}; nl=; fi
    out=${out//(#b)\\([^abefnrtvxXuUc0-7\\])/\\\\$match[1]}
    while [[ $out == (#b)(*)\\0([0-7](#c1,3))(*) ]]; do
      out=$match[1]\\x${(l:2::0:)${$(( [##16] 8#$match[2] ))#16#}}$match[3]
    done
  fi
  if [[ -n $esc ]]; then print -n -- "$out"; else print -rn -- "$out"; fi
  [[ -n $nl ]] && print
  return 0
}
__fish_ret() { return $1 }
_() { print -r -- "$*" }
__fish_builtin_q() { local n; for n; do [[ -n ${(M)${(f)"$(__fish_builtin_names)"}:#$n} ]] || return 1; done }
__fish_command_query() {
  local quiet= all= n r=0 a ch
  local -a hits
  while [[ ${1-} == -* && ${1-} != -- ]]; do
    a=$1; shift
    for ch in ${(s::)a#-}; do case $ch in q) quiet=1 ;; a) all=1 ;; esac; done
  done
  [[ ${1-} == -- ]] && shift
  for n; do
    hits=(${(f)"$(whence -p ${all:+-a} -- $n 2>/dev/null)"})
    (( ${#hits} )) || { r=127; continue }
    [[ -n $quiet ]] && continue
    if [[ -n $all ]]; then print -rl -- $hits; else print -r -- $hits[1]; fi
  done
  return $r
}
# `$name[tokens]`: the elements fish's parse_slice (expand.rs) selects, stored in the
# array DEST. Usage: __fish_index DEST NAME 'TOKENS', or __fish_index DEST - 'TOKENS'
# VALUE… for a list that is not a named parameter ($argv). Negative indices count from
# the end; an index outside the list selects nothing; `a..b` runs in either direction,
# forced backwards when only `a` is negative and forwards when only `b` is, otherwise
# clamped to the last element (a range wholly past the end is skipped); an empty `a` is
# 1 for the first index, an empty `b` is -1.
__fish_index() {
  local __dest=$1 __name=$2 __s=$3 __t __a __b __n __i __j __d
  local -a __toks __src __idx __out
  # fish lets whitespace stand on either side of `..` (`[2 ..3]`, `[1.. ]`)
  while [[ $__s == *" .."* ]]; do __s=${__s// ../..}; done
  while [[ $__s == *".. "* ]]; do __s=${__s//.. /..}; done
  __toks=(${=__s})
  if [[ $__name == - ]]; then
    __src=("${@:4}")
  elif (( ${(P)+__name} )); then
    __src=("${(@P)__name}")
  fi
  __n=$#__src
  for __t in $__toks; do
    if [[ $__t == *..* ]]; then
      __a=${__t%%..*} __b=${__t#*..}
      [[ -z $__a && $#__idx == 0 ]] && __a=1
      [[ -z $__b ]] && __b=-1
      [[ $__a =~ '^[-+]?[0-9]+$' && $__b =~ '^[-+]?[0-9]+$' ]] || continue
      (( __i = __a < 0 ? __n + __a + 1 : __a, __j = __b < 0 ? __n + __b + 1 : __b ))
      (( __i > __n && __j > __n )) && continue
      if (( (__a < 0) != (__b < 0) )); then
        (( __d = __b < 0 ? 1 : -1 ))
      else
        (( __d = __j < __i ? -1 : 1, __i = __i > __n ? __n : __i, __j = __j > __n ? __n : __j ))
      fi
      for (( ; __i * __d <= __j * __d; __i += __d )); do __idx+=($__i); done
    elif [[ $__t =~ '^[-+]?[0-9]+$' ]]; then
      (( __i = __t < 0 ? __n + __t + 1 : __t ))
      __idx+=($__i)
    fi
  done
  for __i in $__idx; do
    (( __i >= 1 && __i <= __n )) && __out+=("${__src[__i]}")
  done
  typeset -ga $__dest
  builtin eval "$__dest=(\"\${(@)__out}\")"
}
typeset -gA __fish_feature_flags=(stderr-nocaret on qmark-noglob on regex-easyesc on ampersand-nobg-in-token on remove-percent-self off test-require-arg off mark-prompt on ignore-terminfo on query-term on omit-term-workarounds off)
__fish_features() {
  command cat <<'__FISH_EOF'
stderr-nocaret          on  3.0 ^ no longer redirects stderr (historical, can no longer be changed)
qmark-noglob            on  3.0 ? no longer globs
regex-easyesc           on  3.1 string replace -r needs fewer \'s
ampersand-nobg-in-token on  3.4 & only backgrounds if followed by a separator
remove-percent-self     off 4.0 %self is no longer expanded (use $fish_pid)
test-require-arg        off 4.0 builtin test requires an argument
mark-prompt             on  4.0 write OSC 133 prompt markers to the terminal
ignore-terminfo         on  4.1 do not look up $TERM in terminfo database (historical, can no longer be changed)
query-term              on  4.1 query the TTY to enable extra functionality
omit-term-workarounds   off 4.3 skip workarounds for incompatible terminals
__FISH_EOF
}
status() {
  case ${1-} in
    is-interactive|-i|--is-interactive) [[ -o interactive ]] ;;
    is-login|-l|--is-login) [[ -o login ]] ;;
    is-command-substitution|-c) (( ZSH_SUBSHELL )) ;;
    is-block|is-breakpoint|is-event|-b) return 1 ;;
    job-control|-j) (( $# > 1 )) && return 0; return 1 ;;
    features) __fish_features ;;
    test-feature) [[ -n ${__fish_feature_flags[$2]-} ]] || return 2; [[ ${__fish_feature_flags[$2]} == on ]] ;;
    filename|-f|--current-filename) print -r -- ${ZSH_ARGZERO:-Standard input} ;;
    current-command) print -r -- ${funcstack[2]:-fish} ;;
    current-function|function) print -r -- ${funcstack[2]:-Not a function} ;;
    fish-path) print -r -- '@ZSHRS@' ;;
    current-line-number|line-number|-n) print -r -- $LINENO ;;
    basename) print -r -- ${ZSH_ARGZERO:t} ;;
    dirname) print -r -- ${ZSH_ARGZERO:A:h} ;;
    stack-trace|print-stack-trace|buildinfo|terminal-os) return 0 ;;
    '') [[ -o interactive ]] && print -r -- 'This is an interactive session' || print -r -- 'This is a non-interactive session' ;;
    *) print -u2 -r -- "status $1: invalid subcommand"; return 2 ;;
  esac
}
__fish_set_list() {
  # locals are `__`-prefixed: they show up in $parameters while this runs and are skipped
  local __sc= __nm= __ex= __lg= __n __k __line __v __t
  local -a __vals __out __in
  local __hasin=
  for __k; do [[ $__k == --in=* ]] && __hasin=1; done
  for __k; do
    case $__k in
      --global|--universal) __sc=g ;;
      --local) __sc=l ;;
      --names) __nm=1 ;;
      --long) __lg=1 ;;
      --in=*) __in=(${=__k#--in=}) ;;
      --export) __ex=1 ;;
      --*) ;;
      -*) for __v in ${(s::)__k#-}; do
            case $__v in g|U) __sc=g ;; l) __sc=l ;; n) __nm=1 ;; L) __lg=1 ;; x) __ex=1 ;; esac
          done ;;
    esac
  done
  for __n in ${(ok)parameters}; do
    __k=$__n
    case $__n in
      _fl_top_*) __k=${__n#_fl_top_}; [[ $__sc == g ]] && continue; [[ -n $__hasin ]] && continue ;;
      _fl<->_*) __k=${__n#_fl<->_}; [[ $__sc == g ]] && continue; [[ -n $__hasin && -z ${__in[(r)$__n]} ]] && continue ;;
      __fx_*) continue ;;
      # this function's own locals and the shell's scratch variables
      __sc|__nm|__ex|__lg|__n|__k|__line|__v|__t|__vals|__out|__in|__hasin|__virtual|__fn|__fv) continue ;;
      __fx_*|__fc<->|__fa<->|__fi<->_<->|__fish_(cs|ps|rc|nrc|nq|av|nav|srcfile|argv|argv_opts|argflags|ipf|ips)) continue ;;
      # every other `__` name is a user's (fish's own scripts set many), bar what this file made
      __fish_uvar_*|__fish_(bin|cache|config|data|help|man|sysconf|user_data)_dir) continue ;;
      __*) (( ${+__fish_internal[$__n]} )) && continue; [[ $__sc == l ]] && continue ;;
      _fish_x|_fl*|_fish_v_*|.*) continue ;;
      # zsh's own parameters, which fish does not have
      CPUTYPE|HOST|INFOPATH|KEYTIMEOUT|LISTMAX|LOGNAME|MACHTYPE|MAILCHECK|OSTYPE|TIMEFMT|TMPPREFIX|TTY|VENDOR|ZSH_*|signals) continue ;;
      chpwd_functions|precmd_functions|preexec_functions|zshexit_functions|periodic_functions|*_functions) continue ;;
      dirprev|dirnext) (( ${#${(P)__n}} )) || continue ;;
      PWD|SHLVL|OLDPWD) [[ -n $__ex ]] || continue ;;
      path) __k=PATH; [[ $__sc == l ]] && continue ;;
      cdpath) __k=CDPATH; [[ $__sc == l ]] && continue ;;
      manpath) __k=MANPATH; [[ $__sc == l ]] && continue ;;
      *) [[ $__sc == l ]] && continue ;;
    esac
    __t=${(Pt)__n}
    [[ $__t == *association* || ( $__t == *special* && $__t != *export* ) ]] && continue
    [[ -n $__ex && $__t != *export* ]] && continue
    if [[ -n $__nm ]]; then __out+=($__k); continue; fi
    # the path variables are lists in fish: their elements, from the tied arrays
    if [[ $__n == (PATH|CDPATH|MANPATH) ]]; then __vals=("${(@P)${(L)__n}}")
    elif (( ${+parameters[__fx_$__n]} )); then __vals=("${(@P)${:-__fx_$__n}}")   # an exported list
    elif [[ $__t == array* ]]; then __vals=("${(@P)__n}")
    else __vals=("${(P)__n}"); fi
    __line=$__k
    if (( ${#__vals} == 1 )); then
      [[ -n ${__vals[1]} ]] && __line+=" ${(q-)__vals[1]}"
    else
      for __v in "${__vals[@]}"; do __line+=" '${__v//\'/\\\'}'"; done
    fi
    # `set` shortens a long value to 60 characters and an ellipsis; `set -L` does not
    if [[ -z $__lg ]] && (( ${#__line} - ${#__k} - 1 > 64 )); then
      __line="$__k ${__line[$(( ${#__k} + 2 )),$(( ${#__k} + 61 ))]}…"
    fi
    __out+=($__line)
  done
  # a function's own `argv` is always local to it
  if [[ -n $__hasin ]]; then
    if (( ${#__fish_av} == 1 )); then __out+=("argv ${(q-)__fish_av[1]}")
    elif (( ${#__fish_av} )); then
      __line=argv; for __v in "${__fish_av[@]}"; do __line+=" '${__v//\'/\\\'}'"; done; __out+=($__line)
    else __out+=(argv); fi
    __out=("${(@o)__out}")
  fi
  if [[ $__sc != l ]]; then
    # variables fish has that no zsh parameter stands for
    local -A __virtual=(
      _ '' __fish_bin_dir ${__fish_bin_dir[1]} __fish_cache_dir ${__fish_cache_dir[1]}
      __fish_config_dir ${__fish_config_dir[1]} __fish_data_dir ${__fish_data_dir[1]}
      __fish_help_dir ${__fish_help_dir[1]} __fish_man_dir ${__fish_man_dir[1]}
      __fish_sysconf_dir ${__fish_sysconf_dir[1]} __fish_user_data_dir ${__fish_user_data_dir[1]}
      argv '' fish_bind_mode default fish_history '' fish_killring '' fish_pid $$
      fish_private_mode 1 fish_terminal_color_theme '' history '' hostname ${HOST%%.*}
      pipestatus 0 status 0 status_generation 0 COLUMNS ${COLUMNS:-80} LINES ${LINES:-24}
      EUID $EUID IFS x
    )
    for __k in ${(k)__virtual}; do
      [[ -n $__ex ]] && continue
      (( ${+__out[(r)$__k]} )) && continue
      [[ -n ${(M)__out:#$__k( *|)} ]] && continue
      if [[ -n $__nm ]]; then __out+=($__k)
      elif [[ $__k == IFS ]]; then __out+=('IFS \n\ \t')
      elif [[ -n ${__virtual[$__k]} ]]; then __line="$__k ${(q-)__virtual[$__k]}"
        if [[ -z $__lg ]] && (( ${#__line} - ${#__k} - 1 > 64 )); then __line="$__k ${__line[$(( ${#__k} + 2 )),$(( ${#__k} + 61 ))]}…"; fi
        __out+=($__line)
      else __out+=($__k); fi
    done
    # fish lists SHLVL among the exported variables (no value) even when it
    # inherited none and a non-interactive shell left it unset
    [[ -n $__ex ]] && (( ! ${+SHLVL} )) && __out+=(SHLVL)
    __out=("${(@o)__out}")
  fi
  (( ${#__out} )) && print -rl -- "${(@)__out}"
  return 0
}
__fish_set_show() {
  local z=$1 n=$2 t kind=global exp=unexported cnt
  local -a vals
  [[ ${+parameters[$z]} == 1 ]] || return 1
  t=${(Pt)z}
  [[ $z == _fl* ]] && kind=local
  [[ $t == *export* || $z == __fx_* ]] && exp=exported
  if [[ $t == array* ]]; then vals=("${(@P)z}")
  elif [[ $exp == exported ]]; then
    # an exported list is one joined string in the environment
    if [[ $n == *PATH ]] || (( ${+__fish_pathvars[$n]} )); then vals=("${(@s.:.)${(P)z}}"); else vals=(${=${(P)z}}); fi
  else vals=("${(P)z}"); fi
  local what="with ${#vals} elements" orig
  case $n in
    PATH|CDPATH|MANPATH)
      [[ ${(Pt)n} == *export* || ${(t)PATH} == *export* ]] && exp=exported
      what="a path variable with ${#vals} elements" ;;
    *) (( ${+__fish_pathvars[$n]} )) && what="a path variable with ${#vals} elements" ;;
  esac
  print -r -- "\$${n}: set in $kind scope, $exp, $what"
  local i
  for (( i = 1; i <= ${#vals}; i++ )); do print -r -- "\$${n}[$i]: |${vals[i]}|"; done
  [[ $exp == exported ]] && orig=$(__fish_inherited $n) && print -r -- "\$${n}: originally inherited as |$orig|"
  return 0
}
typeset -gA __fish_abbrs __fish_fnsrc __fish_fnline __fish_fnfile __fish_aliases __fish_fndesc __fish_fnshadow
# fish variable name -> zsh parameter, for names only known at run time (`$$name`)
typeset -gA __fish_zmap=(PATH path CDPATH cdpath MANPATH manpath hostname HOST)
for __n in path cdpath fpath manpath mailpath module_path signals options commands functions aliases parameters history widgets keymaps modules nameddirs userdirs galiases saliases funcstack jobtexts builtins reswords status prompt psvar watch; do
  [[ -n ${__fish_zmap[$__n]+x} ]] || __fish_zmap[$__n]=_fish_v_$__n
done
unset __n
# variables fish always has that no zsh parameter stands for: readable through `$$name`
typeset -ga __fish_virt_status_generation=(0) __fish_virt_fish_killring=() __fish_virt_history=() __fish_virt_fish_bind_mode=(default) __fish_virt_fish_private_mode=(1) __fish_virt_fish_history=('') __fish_virt_IFS=($'\n \t')
__fish_zmap+=(fish_history __fish_virt_fish_history IFS __fish_virt_IFS status '?' status_generation __fish_virt_status_generation fish_killring __fish_virt_fish_killring history __fish_virt_history fish_bind_mode __fish_virt_fish_bind_mode fish_private_mode __fish_virt_fish_private_mode)
(( COLUMNS )) || COLUMNS=80
(( LINES )) || LINES=24
typeset -ga __fish_abbr_order
__fish_abbr() {
  local op=show pos=command a name exp
  local -a args
  while (( $# )); do
    a=$1; shift
    case $a in
      -a|--add) op=add ;; -e|--erase) op=erase ;; -l|--list) op=list ;;
      -s|--show) op=show ;; -q|--query) op=query ;; -r|--rename) op=rename ;;
      -p|--position) pos=$1; shift ;; --position=*) pos=${a#*=} ;;
      -f|--function|--regex|--command|-c) shift ;;
      --set-cursor|--set-cursor=*) ;;
      --) args+=("$@"); break ;;
      -*) ;;
      *) args+=("$a") ;;
    esac
  done
  [[ $op == show ]] && (( ${#args} )) && op=add
  case $op in
    add)
      (( ${#args} >= 2 )) || { print -u2 -r -- 'abbr --add: Requires at least two arguments'; return 2 }
      name=$args[1]; exp="${(j: :)args[2,-1]}"
      [[ -n ${__fish_abbrs[$name]+x} ]] || __fish_abbr_order+=($name)
      __fish_abbrs[$name]="$pos"$'\x1f'"$exp"
      if [[ $pos == anywhere ]]; then alias -g -- "$name=$exp"; else alias -- "$name=$exp"; fi ;;
    show)
      for name in "${__fish_abbr_order[@]}"; do
        pos=${__fish_abbrs[$name]%%$'\x1f'*} exp=${__fish_abbrs[$name]#*$'\x1f'}
        if [[ $pos == anywhere ]]; then print -r -- "abbr -a --position anywhere -- ${(q-)name} ${(q-)exp}"
        else print -r -- "abbr -a -- ${(q-)name} ${(q-)exp}"; fi
      done ;;
    list) (( ${#__fish_abbr_order} )) && print -rl -- "${__fish_abbr_order[@]}" ;;
    query)
      (( ${#args} )) || return 1
      for name in "${args[@]}"; do [[ -n ${__fish_abbrs[$name]+x} ]] || return 1; done ;;
    erase)
      for name in "${args[@]}"; do
        [[ -n ${__fish_abbrs[$name]+x} ]] || { print -u2 -r -- "abbr --erase: No abbreviation named $name"; continue }
        unset "__fish_abbrs[$name]"
        __fish_abbr_order=("${(@)__fish_abbr_order:#$name}")
        unalias -- "$name" 2>/dev/null
      done ;;
    rename)
      (( ${#args} == 2 )) || { print -u2 -r -- 'abbr --rename: Requires exactly two arguments'; return 2 }
      [[ -n ${__fish_abbrs[$args[1]]+x} ]] || { print -u2 -r -- "abbr --rename: No abbreviation named $args[1]"; return 2 }
      pos=${__fish_abbrs[$args[1]]%%$'\x1f'*} exp=${__fish_abbrs[$args[1]]#*$'\x1f'}
      __fish_abbr --erase "$args[1]"
      __fish_abbr --add --position $pos -- "$args[2]" $=exp ;;
  esac
  return 0
}
# `set NAME[idx…] value…` (MODE `set`; `slice` for -a/-p, which fish refuses) and
# `set -e NAME[idx…]` (MODE `erase`), as fish's builtins/set.rs does them:
# split_var_and_indexes reads the index list against the current length (negative
# counts from the end, `a..b` runs either way, `..b` / `a..` are open only as the
# first index), new_var_values_by_index pads with empty elements, erased_at_indexes
# drops the indexes in range. The values to store are in __fish_iv; HAD is ${+NAME},
# then come the current elements; the new list is left in __fish_io.
__fish_set_idx() {
  local __m=$1 __s="${2//:/ }]" __had=$3 __p=1 __n __l __r __k __bad=
  shift 3
  local -a __cur=("$@") __ix=()
  local __len=${#__cur}
  while [[ ${__s[__p]} != ']' ]]; do
    if (( ! ${#__ix} )) && [[ ${__s[__p,__p+1]} == .. ]]; then
      __l=1
    else
      __fish_set_idx_int || { __bad=${__s[__p,-1]}; break; }
      __l=$__n
    fi
    (( __l < 0 )) && (( __l += __len + 1 ))
    if [[ ${__s[__p,__p+1]} == .. ]]; then
      (( __p += 2 ))
      if (( ! ${#__ix} )) && [[ ${__s[__p]} == ']' ]]; then
        __r=$__len
      else
        __fish_set_idx_int || { __bad=${__s[__p,-1]}; break; }
        __r=$__n
        (( __r < 0 )) && (( __r += __len + 1 ))
      fi
      if (( __l < __r )); then
        for (( __k = __l; __k <= __r; __k++ )); do __ix+=($__k); done
      else
        for (( __k = __l; __k >= __r; __k-- )); do __ix+=($__k); done
      fi
    else
      __ix+=($__l)
    fi
  done
  if [[ -n $__bad ]]; then
    print -u2 -r -- "set: Invalid index starting at '$__bad'"
    [[ $__m == erase ]] && return 1
    return 2
  fi
  if [[ $__m == erase ]]; then
    (( __had )) || return 1
    for __k in ${(Onu)__ix}; do (( __k >= 1 && __k <= __len )) && __cur[__k]=(); done
    __fish_io=("${__cur[@]}")
    return 0
  fi
  for __k in $__ix; do
    (( __k > 0 )) || { print -u2 -r -- 'set: array index out of bounds'; return 2; }
  done
  [[ $__m == slice ]] && { print -u2 -r -- 'set: Cannot use --append or --prepend when assigning to a slice'; return 2; }
  if (( ${#__ix} != ${#__fish_iv} )); then
    print -u2 -r -- "set: given ${#__ix} indexes but ${#__fish_iv} values"
    return 2
  fi
  for (( __k = 1; __k <= ${#__ix}; __k++ )); do
    __l=${__ix[__k]}
    while (( ${#__cur} < __l )); do __cur+=(''); done
    __cur[__l]=${__fish_iv[__k]}
  done
  __fish_io=("${__cur[@]}")
}
# One integer of an index list at __s[__p] into __n (fish's wcstoi_partial: blanks,
# a sign, digits), then the blanks after it; status 1 when there is none.
__fish_set_idx_int() {
  local __st __d __sg=
  while [[ ${__s[__p]} == [[:space:]] ]]; do (( __p++ )); done
  __st=$__p
  [[ ${__s[__p]} == [-+] ]] && { __sg=${__s[__p]}; (( __p++ )); }
  __d=$__p
  while [[ ${__s[__p]} == [0-9] ]]; do (( __p++ )); done
  (( __p > __d )) || { __p=$__st; return 1; }
  __n=$(( 10#${__s[__d,__p-1]} ))
  [[ $__sg == - ]] && __n=$(( -__n ))
  while [[ ${__s[__p]} == [[:space:]] ]]; do (( __p++ )); done
  return 0
}
__fish_set_dynamic() {
  local exp= erase= query= app= pre= n sep=' '
  local o ch
  while [[ ${1-} == -* ]]; do
    o=$1
    case $o in
      --export) exp=1 ;;
      --erase) erase=1 ;;
      --query) query=1 ;;
      --append) app=1 ;;
      --prepend) pre=1 ;;
      --) shift; break ;;
      --*) ;;
      *) for ch in ${(s::)o#-}; do
           case $ch in x) exp=1 ;; e) erase=1 ;; q) query=1 ;; a) app=1 ;; p) pre=1 ;; esac
         done ;;
    esac
    shift
  done
  n=$1; shift
  [[ -n $n ]] || return 1
  # the variables fish always has, bar `_` and the terminal colour theme
  if [[ -n $query ]]; then
    case $n in
      status|status_generation|fish_killring|fish_history|fish_bind_mode|fish_private_mode|history|pipestatus|hostname|fish_pid|COLUMNS|LINES|EUID|IFS) return 0 ;;
      _|fish_terminal_color_theme) return 1 ;;
    esac
  fi
  case $n in
    PATH) n=path ;; CDPATH) n=cdpath ;; MANPATH) n=manpath ;; hostname) n=HOST ;;
    path|cdpath|fpath|manpath|mailpath|module_path|signals|options|commands|functions|aliases|parameters|history|widgets|keymaps|modules|nameddirs|userdirs|galiases|saliases|funcstack|jobtexts|builtins|reswords|status|prompt|psvar|watch) n=_fish_v_$n ;;
  esac
  [[ $n == *PATH ]] && sep=:
  if [[ -n $query ]]; then (( ${+parameters[$n]} )); return; fi
  if [[ -n $erase ]]; then unset "$n"; return 0; fi
  if [[ -n $exp ]]; then
    local -a __v=("$@")
    [[ ${(Pt)n} == array* ]] && unset "$n"
    typeset -gx "$n=${(pj:$sep:)__v}"
    return 0
  fi
  [[ -n ${parameters[$n]} && ${(Pt)n} != array* ]] && unset "$n"
  typeset -ga "$n"
  if [[ -n $app ]]; then builtin eval "$n+=(\"\$@\")"
  elif [[ -n $pre ]]; then builtin eval "$n=(\"\$@\" \"\${(@)$n}\")"
  else builtin eval "$n=(\"\$@\")"; fi
}
type() {
  local q= t= a n k r=0 pth=
  for a; do
    case $a in
      -q|--quiet) q=1 ;;
      -t|--type) t=1 ;;
      -P|--path) pth=1 ;;
      -*) ;;
      *) n=$a
         if [[ -n $pth ]]; then
           k=$(whence -p -- $n 2>/dev/null)
           [[ -n $k ]] && { [[ -n $q ]] || print -r -- $k } || r=1
           continue
         fi
         k=$(whence -w -- $n 2>/dev/null); k=${k##*: }
         [[ $k == reserved || $n == "begin" ]] && k=builtin
         [[ -n $k && $k != none ]] || { r=1; [[ -n $q ]] || print -u2 -r -- "type: Could not find '$n'"; continue }
         [[ -n $q ]] && continue
         case $k in
           function)
             if [[ -n $t ]]; then print -r -- function
             elif [[ -n ${__fish_fnsrc[$n]+x} ]]; then
               print -r -- "$n is a function with definition"
               __fish_defined_in $n
               print -r -- ${__fish_fnsrc[$n]}
             else print -r -- "$n is a function"; fi ;;
           builtin) [[ -n $t ]] && print -r -- builtin || print -r -- "$n is a builtin" ;;
           command) [[ -n $t ]] && print -r -- file || print -r -- "$n is ${$(whence -p -- $n)}" ;;
           *) [[ -n $t ]] && print -r -- $k || print -r -- "$n is a $k" ;;
         esac ;;
    esac
  done
  return $r
}
# fish remembers an erased function: it is not autoloaded again
typeset -gA __fish_erased
# does `(NAME …)` have to run in this shell? only if NAME sets a global; a name nothing
# defines yet is autoloaded first, as fish would
__fish_inproc() {
  if (( ! ${+functions[$1]} && ! ${+commands[$1]} && ! ${+builtins[$1]} )); then __fish_autoload_ok $1 2>/dev/null; fi
  __fish_gsets $1
}
# does function $1 set a global, itself or through a function it calls?
typeset -gA __fish_gtrans
__fish_gsets() {
  local n=$1 w
  (( ${+__fish_gset[$n]} || ${+__fish_gtrans[$n]} )) && return 0
  (( ${2:-0} > 4 )) && return 1
  for w in ${=${__fish_fnsrc[$n]//[^A-Za-z0-9_]/ }}; do
    [[ $w == $n ]] && continue
    (( ${+functions[$w]} )) && __fish_gsets $w $(( ${2:-0} + 1 )) && { __fish_gtrans[$n]=1; return 0 }
  done
  return 1
}
__fish_autoload_ok() { [[ -z ${__fish_erased[$1]+x} ]] && __fish_autoload "$1" }
# fish's `which` finds files only; `umask` prints four octal digits
which() {
  local n p r=0
  for n; do
    p=$(whence -p -- $n 2>/dev/null) && print -r -- $p || r=1
  done
  return $r
}
umask() {
  local m
  case ${1-} in
    "") builtin umask | { read -r m; print -r -- ${(l:4::0:)m} } ;;
    -p|--as-command) print -r -- "umask ${(l:4::0:)$(builtin umask)}" ;;
    -S|--symbolic) builtin umask -S ;;
    -*) builtin umask "$@" ;;
    *) [[ $1 == <-> || $1 == [ugoa]*=* || $1 == [-+ugorwxa,=]# ]] || { print -u2 -r -- "umask: Invalid mask '$1'"; return 1 }
       builtin umask -- "$@" ;;
  esac
}
functions() {
  local a n=() q= e= listnames= alln= details= verbose= miss=0 args=() i cp= dsc= setdesc=
  # fish's getopt takes short flags together (`functions -na`)
  for a; do
    if [[ $a == -[^-]?* ]]; then
      for (( i = 2; i <= $#a; i++ )); do args+=(-$a[i]); done
    else
      args+=($a)
    fi
  done
  local nx=
  for a in $args; do
    if [[ -n $nx ]]; then setdesc=$a; nx=; continue; fi
    case $a in
      -d|--description) dsc=1; nx=1 ;;
      -q|--query) q=1 ;;
      -e|--erase) e=1 ;;
      -c|--copy) cp=1 ;;
      -D|--details) details=1 ;;
      -v|--verbose) verbose=1 ;;
      --description=*) dsc=1; setdesc=${a#--description=} ;;
      -n|--names) listnames=1 ;;
      -a|--all) alln=1 ;;
      -*) ;;
      *) n+=($a) ;;
    esac
  done
  if [[ -n $dsc ]]; then
    (( ${#n} == 1 )) || { print -u2 -r -- 'functions: Expected exactly one function name'; return 2 }
    (( $+functions[$n[1]] )) || __fish_autoload_ok $n[1] || { print -u2 -r -- "functions: Function '$n[1]' does not exist"; return 1 }
    __fish_fndesc[$n[1]]=$setdesc
    if [[ -n ${__fish_fnsrc[$n[1]]+x} ]]; then
      local __h=${__fish_fnsrc[$n[1]]%%$'\n'*} __r=${__fish_fnsrc[$n[1]]#*$'\n'}
      local __re=" --description ('[^']*'|[^ ]+)"
      [[ $__h =~ $__re ]] && __h=${__h/$MATCH/}
      [[ -n $setdesc ]] && __h+=" --description ${(q-)setdesc}"
      __fish_fnsrc[$n[1]]=$__h$'\n'$__r
    fi
    return 0
  fi
  if [[ -n $listnames ]]; then
    if [[ $(whence -w __fish_function_names 2>/dev/null) != *none* && -n $(whence -w __fish_function_names 2>/dev/null) ]]; then __fish_function_names ${alln:+-a}; else print -rl -- ${(ok)functions}; fi
    return
  fi
  # fish autoloads each name (get_props_autoload); -q returns how many are missing
  if [[ -n $q ]]; then
    (( ${#n} )) || return 1
    for a in $n; do (( $+functions[$a] )) || __fish_autoload_ok $a || (( ++miss )); done
    return miss
  fi
  if [[ -n $cp ]]; then
    (( ${#n} == 2 )) || { print -u2 -r -- "functions: Expected exactly two names (current function name, and new function name)"; return 1 }
    (( $+functions[$n[1]] )) || __fish_autoload_ok $n[1] || { print -u2 -r -- "functions: Function '$n[1]' does not exist"; return 1 }
    (( $+functions[$n[2]] )) && { print -u2 -r -- "functions: Function '$n[2]' already exists. Cannot create copy '$n[1]'"; return 1 }
    builtin functions -c -- $n[1] $n[2] || return 1
    if [[ -n ${__fish_fnsrc[$n[1]]+x} ]]; then
      __fish_fnsrc[$n[2]]=${__fish_fnsrc[$n[1]]/function $n[1]/function $n[2]}
      __fish_fnline[$n[2]]=${__fish_fnline[$n[1]]-1}
      __fish_fnfile[$n[2]]=${__fish_fnfile[$n[1]]-}
    fi
    return 0
  fi
  if [[ -n $e ]]; then for a in $n; do unfunction -- $a 2>/dev/null; __fish_erased[$a]=1; unset "__fish_aliases[$a]" "__fish_fndesc[$a]" "__fish_fnshadow[$a]" "__fish_fnsrc[$a]" "__fish_fnline[$a]" "__fish_fnfile[$a]" 2>/dev/null; done; return 0; fi
  if (( ! ${#n} )); then print -rl -- ${(ko)functions}; return 0; fi
  for a in $n; do
    if [[ -n $details ]] && ! { (( $+functions[$a] )) || __fish_autoload_ok $a }; then
      if [[ -n $verbose ]]; then print -rl -- n/a n/a 0 n/a n/a; else print -r -- n/a; fi
      continue
    fi
    (( $+functions[$a] )) || __fish_autoload_ok $a || { print -u2 -r -- "functions: Unknown function '$a'"; return 1 }
    if [[ -n ${__fish_fnsrc[$a]+x} ]]; then
      if [[ -n $details ]]; then
        if [[ -n $verbose ]]; then
          print -r -- ${__fish_fnfile[$a]:-stdin}$'\nnot-autoloaded\n'${__fish_fnline[$a]:-0}
          print -r -- ${${__fish_fnshadow[$a]:+no-scope-shadowing}:-scope-shadowing}$'\n'${__fish_fndesc[$a]}
        else print -r -- ${__fish_fnfile[$a]:-stdin}; fi
        continue
      fi
      __fish_defined_in $a
      print -r -- ${__fish_fnsrc[$a]}
    else
      builtin functions -- $a
    fi
  done
}
funcsave() {
  local a dir=${XDG_CONFIG_HOME:-$HOME/.config}/fish/functions
  (( $# )) || { print -u2 -r -- 'funcsave: Expected exactly one function name'; return 1 }
  for a; do
    [[ -n ${__fish_fnsrc[$a]+x} ]] || { print -u2 -r -- "funcsave: Unknown function '$a'"; return 1 }
    [[ -d $dir ]] || mkdir -p $dir
    print -r -- ${__fish_fnsrc[$a]} > $dir/$a.fish
    print -r -- "funcsave: wrote $dir/$a.fish"
  done
}
funced() {
  local ed= save= n tmpdir tmp wp= sum new ans which st=0 init
  local -a edw
  while [[ ${1-} == -* ]]; do
    case $1 in
      -e|--editor) ed=$2; shift ;;
      --editor=*) ed=${1#--editor=} ;;
      -s|--save) save=1 ;;
      -i|--interactive) ed=fish ;;
      -h|--help) return 0 ;;
    esac
    shift
  done
  (( $# )) || { print -r -- 'funced: Expected at least 1 args, got only 0'; return 1 }
  (( $# == 1 )) || { print -u2 -r -- 'funced: expected <= 1 arguments; got '$#; return 1 }
  n=$1
  if [[ -z $ed ]]; then
    if [[ -n ${VISUAL+x} ]]; then ed=$VISUAL; which=VISUAL
    elif [[ -n ${EDITOR+x} ]]; then ed=$EDITOR; which=EDITOR
    else ed=fish; fi
  fi
  edw=(${=ed})
  if [[ $ed != fish ]] && ! (( $+commands[$edw[1]] || $+builtins[$edw[1]] )) && ! [[ $edw[1] == */* && -x $edw[1] ]]; then
    print -u2 -r -- "funced: The value for \$${which:-EDITOR} '$ed' could not be used because the command '$edw[1]' could not be found"
    ed=fish
  fi
  if [[ $ed == fish ]]; then
    IFS= read -r ans && eval "$ans"
    [[ -n $save ]] && funcsave $n
    return 0
  fi
  tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/fish-funced.XXXXXX") || return 1
  tmp=$tmpdir/$n.fish
  init="function $n"$'\n\nend'
  if (( ! $+functions[$n] )); then
    print -r -- $init > $tmp
  elif [[ -f ${__fish_fnfile[$n]-} ]]; then
    wp=${__fish_fnfile[$n]}
    command cat $wp > $tmp
  else
    functions -- $n > $tmp
  fi
  sum=$(command cksum < $tmp)
  while true; do
    if ! $edw $tmp; then
      print -r -- "Editing failed or was cancelled"
    else
      new=$(command cksum < $tmp)
      if [[ $new == $sum ]]; then
        print -r -- "Editor exited but the function was not modified"
        print -r -- "If the editor is still running, check if it waits for completion, maybe a '--wait' option?"
        [[ -n $wp ]] && source $wp
        break
      fi
      if ! source < $tmp; then
        print
        ans=
        read -r ans || ans=n
        if [[ -z $ans || $ans == [Yy]([Ee][Ss]|) ]]; then continue; fi
        print -r -- "Cancelled function editing"
      elif [[ -n $wp ]]; then
        if [[ -z $save ]]; then
          print -r -- "Warning: the file containing this function has not been saved. Changes may be lost when fish is closed."
          ans=
          read -r ans
          [[ -z $ans || $ans == [Yy]([Ee][Ss]|) ]] && save=1
        fi
        if [[ -n $save ]]; then
          if command cp $tmp $wp 2>&1; then
            print -r -- "Function saved to $wp"
            source $wp
          else
            print -r -- "Saving to original location failed; saving to user configuration instead."
            wp=${__fish_config_dir[1]}/functions/${wp:t}
            if command cp $tmp $wp; then
              print -r -- "Function saved to $wp"
              source $wp
            else
              print -r -- "Saving to user configuration failed. Changes may be lost when fish is closed."
            fi
          fi
        fi
      elif [[ -n $save ]]; then
        funcsave $n
      else
        print -r -- "Run funcsave $n to save this function to the configuration directory."
      fi
    fi
    break
  done
  command rm -f $tmp
  command rmdir $tmpdir 2>/dev/null
  return $st
}
__fish_defined_in() {
  if [[ -n ${__fish_fnline[$1]-} && ! -f ${__fish_fnfile[$1]-} ]]; then
    # defined by `-c` or at the prompt: no file to name
    print -r -- '# Defined interactively'
  elif [[ -n ${__fish_fnline[$1]-} ]]; then
    print -r -- "# Defined in ${__fish_fnfile[$1]} @ line ${__fish_fnline[$1]}"
  else
    print -r -- '# Defined via `source`'
  fi
}
__fish_alias_list() {
  local k
  for k in ${(ko)__fish_aliases}; do print -r -- "alias $k ${(q-)__fish_aliases[$k]}"; done
}
__fish_time_begin() {
  zmodload -F zsh/datetime p:EPOCHREALTIME 2>/dev/null
  __fish_t0=$EPOCHREALTIME
  __fish_ru0=($(__fish_rusage))
}
__fish_time_end() {
  local st=$1 t1=$EPOCHREALTIME
  local -a ru=($(__fish_rusage))
  local -F wall=$(( (t1 - __fish_t0) * 1000000 ))
  local -a row
  __fish_dur() {
    local -F v=$1
    if (( v >= 1000000 )); then printf '%8.2f %-6s' $(( v / 1000000 )) secs
    elif (( v >= 1000 )); then printf '%8.2f %-6s' $(( v / 1000 )) millis
    else printf '%8.2f %-6s' $v micros; fi
  }
  local -F fu=$(( ru[1] - __fish_ru0[1] )) fs=$(( ru[2] - __fish_ru0[2] ))
  local -F eu=$(( ru[3] - __fish_ru0[3] )) es=$(( ru[4] - __fish_ru0[4] ))
  {
    print
    print -r -- ${(l:56::_:)}
    print -r -- "Executed in$(__fish_dur $wall)    fish           external"
    print -r -- "   usr time$(__fish_dur $(( fu + eu )))$(__fish_dur $fu)$(__fish_dur $eu)"
    print -r -- "   sys time$(__fish_dur $(( fs + es )))$(__fish_dur $fs)$(__fish_dur $es)"
    print
  } >&2
  unfunction __fish_dur
  return $st
}
# background jobs by pid -> "number<TAB>command", recorded when the job starts
typeset -gA __fish_jobtab
__fish_jobreg() {
  local n=1 p
  for p in ${(k)__fish_jobtab}; do
    kill -0 $p 2>/dev/null || { unset "__fish_jobtab[$p]"; continue }
    (( ${__fish_jobtab[$p]%%$'\t'*} >= n )) && n=$(( ${__fish_jobtab[$p]%%$'\t'*} + 1 ))
  done
  __fish_jobtab[$1]=$n$'\t'$2
}
# fish's `jobs`: newest first, `Job Group State Command` columns without a header,
# and a status of 1 when nothing is listed
jobs() {
  local mode= q= p e last= a f
  local -a rows=() want=() c
  while [[ ${1-} == -* ]]; do
    case $1 in
      --) shift; break ;;
      -p|--pid) mode=p ;;
      -g|--group) mode=g ;;
      -c|--command) mode=c ;;
      -l|--last) last=1 ;;
      -q|--query) q=1 ;;
      -h|--help) return 0 ;;
    esac
    shift
  done
  for p e in ${(kv)__fish_jobtab}; do
    kill -0 $p 2>/dev/null && rows+=("${e%%$'\t'*}"$'\t'$p$'\t'"${e#*$'\t'}")
  done
  rows=(${(On)rows})
  for a in "$@"; do
    f=(${(M)rows:#*$'\t'$a$'\t'*})
    (( ${#f} )) || { [[ -n $q ]] || print -u2 -r -- "jobs: No suitable job: $a"; return 1 }
    want+=($f)
  done
  (( $# )) && rows=($want)
  (( ${#rows} )) || { [[ -n $q || $# -gt 0 ]] || (( ZSH_SUBSHELL )) || print -r -- "jobs: There are no jobs"; return 1 }
  [[ -n $q ]] && return 0
  [[ -n $last ]] && rows=($rows[1])
  [[ $mode == c ]] && ! (( ZSH_SUBSHELL )) && print -r -- Command
  [[ -z $mode ]] && ! (( ZSH_SUBSHELL )) && print -r -- $'Job\tGroup\tState\tCommand'
  for e in $rows; do
    c=("${(@ps:\t:)e}")
    case $mode in
      p) print -r -- $c[2] ;;
      g) print -r -- - ;;
      c) print -r -- ${c[3]%% *} ;;
      *) print -r -- "$c[1]"$'\t'"-"$'\t'"running"$'\t'"$c[3] &" ;;
    esac
  done
  return 0
}
set_color() {
  local a c= bg= r=() esc=$'\e'
  local -A fg=(black 30 red 31 green 32 yellow 33 blue 34 magenta 35 cyan 36 white 37 brblack 90 brred 91 brgreen 92 bryellow 93 brblue 94 brmagenta 95 brcyan 96 brwhite 97 brgrey 90 grey 90)
  local code
  while (( $# )); do
    a=$1; shift
    case $a in
      normal) print -rn -- "${esc}[m"; return 0 ;;
      -o|--bold) r+=(1) ;;
      -d|--dim) r+=(2) ;;
      -i|--italics) r+=(3) ;;
      -u|--underline) r+=(4) ;;
      -r|--reverse) r+=(7) ;;
      -b|--background) bg=$1; shift ;;
      -c|--print-colors) print -rl -- black blue brblack brblue brcyan brgreen brmagenta brred brwhite bryellow cyan green magenta red white yellow normal; return 0 ;;
      -v|--version) ;;
      -*) ;;
      *) c=$a ;;
    esac
  done
  if [[ -n $c ]]; then
    if [[ -n ${fg[$c]} ]]; then r=(${fg[$c]} $r)
    elif [[ $c == (#b)([0-9a-fA-F](#c6)) ]]; then r=("38;2;$((16#${c[1,2]}));$((16#${c[3,4]}));$((16#${c[5,6]}))" $r)
    fi
  fi
  if [[ -n $bg && -n ${fg[$bg]} ]]; then r+=($(( ${fg[$bg]} + 10 ))); fi
  (( ${#r} )) && print -rn -- "${esc}[${(j.;.)r}m"
  return 0
}
fish_add_path() {
  local d new=() p=
  for d; do
    case $d in -p|--prepend) p=1 ;; -*) ;; *) [[ -d $d ]] && (( ! ${path[(Ie)$d]} )) && new+=($d) ;; esac
  done
  [[ -n $p ]] && path=($new $path) || path=($new $path)
  return 0
}
random() {
  case ${1-} in
    choice) shift; (( $# )) || return 1; print -r -- ${@[RANDOM % $# + 1]} ;;
    '') print -r -- $RANDOM ;;
    *) local lo=$1 hi=${2-}; [[ -n $hi ]] || { hi=$lo; lo=0 }; print -r -- $(( lo + RANDOM % (hi - lo + 1) )) ;;
  esac
}
psub() {
  local f
  f=$(mktemp "${TMPDIR:-/tmp}/.psub.XXXXXX") || return 1
  cat > $f
  print -r -- $f
}
isatty() { local fd=${1-1}; case $fd in stdin) fd=0;; stdout) fd=1;; stderr) fd=2;; esac; [[ -t $fd ]] }
dirprev=()
dirnext=()
__fish_chpwd() {
  [[ -n ${__fish_nav-} ]] && return
  # a cd that stays where it was leaves no history
  [[ $OLDPWD == $PWD ]] && return
  dirprev+=($OLDPWD)
  dirnext=()
  (( ${#dirprev} > 25 )) && dirprev[1]=()
}
chpwd_functions+=(__fish_chpwd)
# `cd -` walks back through the directory history, as `prevd` does
cd() {
  if [[ $# == 1 && $1 == - ]]; then prevd; else builtin cd "$@"; fi
}
prevd() {
  local n=1 d list=
  [[ ${1-} == -l || ${1-} == --list ]] && { list=1; shift }
  [[ ${1-} == <-> ]] && n=$1
  while (( n-- > 0 )); do
    (( ${#dirprev} )) || { print -u2 -r -- 'prevd: Hit end of history'; return 1 }
    d=$dirprev[-1]
    dirprev[-1]=()
    dirnext+=($PWD)
    __fish_nav=1
    builtin cd -- $d
    __fish_nav=
  done
  [[ -n $list ]] && dirh
  return 0
}
nextd() {
  local n=1 d list=
  [[ ${1-} == -l || ${1-} == --list ]] && { list=1; shift }
  [[ ${1-} == <-> ]] && n=$1
  while (( n-- > 0 )); do
    (( ${#dirnext} )) || { print -u2 -r -- 'nextd: Hit end of history'; return 1 }
    d=$dirnext[-1]
    dirnext[-1]=()
    dirprev+=($PWD)
    __fish_nav=1
    builtin cd -- $d
    __fish_nav=
  done
  [[ -n $list ]] && dirh
  return 0
}
dirh() {
  local i
  for (( i = 1; i <= ${#dirprev}; i++ )); do
    print -r -- "$(printf '%2d' $(( ${#dirprev} - i + 1 )))) ${dirprev[i]}"
  done
  print -r -- "${PWD}"$'\e[m'
  for (( i = ${#dirnext}; i >= 1; i-- )); do
    print -r -- "$(printf '%2d' $(( ${#dirnext} - i + 1 )))) ${dirnext[i]}"
  done
  print
}
history() {
  local sub=${1-} term limit= mode=contains a
  case $sub in
    search) shift
      while [[ ${1-} == -* ]]; do
        case $1 in
          -e|--exact) mode=exact ;; -p|--prefix) mode=prefix ;; -c|--contains) mode=contains ;;
          -n|--max) limit=$2; shift ;;
        esac
        shift
      done
      term=${1-}
      local -a hits
      case $mode in
        exact) hits=(${(M)${(Oa)history}:#${term}}) ;;
        prefix) hits=(${(M)${(Oa)history}:#${term}*}) ;;
        *) hits=(${(M)${(Oa)history}:#*${term}*}) ;;
      esac
      [[ -n $limit ]] && hits=(${hits[1,limit]})
      (( ${#hits} )) && print -rl -- $hits ;;
    save) builtin fc -W 2>/dev/null; return 0 ;;
    merge) builtin fc -R 2>/dev/null; return 0 ;;
    clear)
      __fish_history_delete --all
      builtin fc -p
      return 0 ;;
    delete) shift
      local dmode=exact dterm k
      local -a all=() keep=()
      while [[ ${1-} == -* ]]; do
        case $1 in
          -e|--exact) dmode=exact ;; -p|--prefix) dmode=prefix ;; -c|--contains) dmode=contains ;;
          -C|--case-sensitive) ;;
        esac
        shift
      done
      dterm=${1-}
      [[ -n $dterm ]] || return 1
      for k in ${(kon)history}; do all+=("${history[$k]}"); done
      case $dmode in
        exact) keep=("${(@)all:#${(b)dterm}}") ;;
        prefix) keep=("${(@)all:#${(b)dterm}*}") ;;
        *) keep=("${(@)all:#*${(b)dterm}*}") ;;
      esac
      __fish_history_delete --$dmode "$dterm"
      builtin fc -p
      for k in "${keep[@]}"; do print -s -r -- "$k"; done
      return 0 ;;
    '') (( ${#history} )) && print -rl -- ${(Oa)history} ;;
  esac
  return 0
}
typeset -gA __fish_widgets=(
  forward-char forward-char backward-char backward-char forward-word forward-word
  backward-word backward-word beginning-of-line beginning-of-line end-of-line end-of-line
  kill-line kill-line backward-kill-line backward-kill-line kill-word kill-word
  backward-kill-word backward-kill-word delete-char delete-char
  backward-delete-char backward-delete-char accept-autosuggestion forward-char
  history-search-backward history-search-backward history-search-forward history-search-forward
  up-or-search up-line-or-search down-or-search down-line-or-search complete complete-word
  execute accept-line clear-screen clear-screen yank yank kill-whole-line kill-whole-line
  transpose-chars transpose-chars upcase-word up-case-word downcase-word down-case-word
  undo undo redo redo repaint redisplay cancel send-break beginning-of-history beginning-of-history
  end-of-history end-of-history
)
__fish_keyname() {
  case $1 in
    ctrl-?) print -rn -- "^${(U)1[6]}" ;;
    alt-?) print -rn -- $'\e'"${1[5]}" ;;
    up) print -rn -- $'\e[A' ;; down) print -rn -- $'\e[B' ;;
    right) print -rn -- $'\e[C' ;; left) print -rn -- $'\e[D' ;;
    home) print -rn -- $'\e[H' ;; end) print -rn -- $'\e[F' ;;
    delete) print -rn -- $'\e[3~' ;; ppage) print -rn -- $'\e[5~' ;; npage) print -rn -- $'\e[6~' ;;
    backspace) print -rn -- $'\x7f' ;; tab) print -rn -- $'\t' ;;
    enter) print -rn -- $'\r' ;; escape) print -rn -- $'\e' ;;
    *) print -rn -- $1 ;;
  esac
}
integer __fish_nbinds=0
__fish_bind() {
  local erase= key= cmd w body
  local -a pos
  while (( $# )); do
    case $1 in
      -M|--mode|-m|--sets-mode) shift 2 ;;
      -e|--erase) erase=1; shift ;;
      -k|--key) pos+=("$(__fish_keyname $2)"); shift 2 ;;
      --) shift; break ;;
      -*) shift ;;
      *) break ;;
    esac
  done
  if (( ${#pos} )); then key=$pos[1]; else key=$(__fish_keyname "${1-}"); (( $# )) && shift; fi
  [[ -n $key ]] || return 0
  if [[ -n $erase ]]; then bindkey -r -- "$key" 2>/dev/null; return 0; fi
  cmd="$*"
  [[ -n $cmd ]] || return 0
  if [[ -n ${__fish_widgets[$cmd]} ]]; then
    bindkey -- "$key" ${__fish_widgets[$cmd]}
  else
    (( __fish_nbinds++ ))
    w=__fish_bind_$__fish_nbinds
    body=$(print -r -- "$cmd" | '@ZSHRS@' --fish-translate -) || return 1
    builtin eval "$w() { $body
zle reset-prompt }"
    zle -N $w
    bindkey -- "$key" $w
  fi
  return 0
}
__fish_commandline() {
  # editing operations act on the zle buffer; reading is done by
  # __fish_cmdline_read, which answers from the line being completed while a
  # completion runs (fish's transient commandline)
  local a t
  local -a reads
  while (( $# )); do
    a=$1; shift
    case $a in
      -f|--function) t=${__fish_widgets[$1]:-$1}; shift; zle $t 2>/dev/null; return 0 ;;
      -r|--replace) BUFFER=${1-}; CURSOR=${#BUFFER}; return 0 ;;
      -i|--insert) LBUFFER+=${1-}; return 0 ;;
      -a|--append) RBUFFER+=${1-}; return 0 ;;
      -C|--cursor) if (( $# )) && [[ $1 == <-> ]]; then CURSOR=$1; return 0; fi; reads+=($a) ;;
      *) reads+=($a) ;;
    esac
  done
  __fish_cmdline_read "${BUFFER-}" "${CURSOR-0}" "${(@)reads}"
}
typeset -gA __fish_events __fish_pathvars __fish_gset
__fish_on_event() {
  case $1 in
    fish_preexec) preexec_functions+=($2) ;;
    fish_postexec|fish_prompt) precmd_functions+=($2) ;;
    fish_exit) zshexit_functions+=($2) ;;
    *) __fish_events[$1]+="${__fish_events[$1]:+ }$2" ;;
  esac
}
emit() {
  local ev=$1 f
  shift
  for f in ${=__fish_events[$ev]}; do $f "$@"; done
  return 0
}
# `--on-variable NAME`: only PWD can be watched (it changes through chpwd)
__fish_on_variable() {
  __fish_events[var:$1]+="${__fish_events[var:$1]:+ }$2"
  [[ $1 == PWD ]] && (( ${chpwd_functions[(I)__fish_pwd_event]} )) || [[ $1 != PWD ]] || chpwd_functions+=(__fish_pwd_event)
}
__fish_var_event() {
  local f
  for f in ${=__fish_events[var:$1]}; do $f VARIABLE $2 $1; done
}
__fish_pwd_event() {
  local f
  for f in ${=__fish_events[var:PWD]}; do $f VARIABLE SET PWD; done
}
__fish_argparse() {
  local name=${${funcstack[2,-1]:#(argparse|__fish_argparse)}[1]:-argparse} ign= stop= mu= amp= consumed= skipshort= spec s sfx short long id a o val k i c us=$'\x1f'
  local -a vals opts=() excl=()
  typeset -ga __fish_argflags
  __fish_argflags=(argv_opts)
  local intid= intidname= minargs= maxargs= nov=$'\x02' xs xa xb present p
  local -A kind=() alias=() acc=() noopt=() valid=()
  local -a specs=() rest=() ids=()
  while (( $# )); do
    case $1 in
      -n|--name) name=$2; shift 2 ;;
      -x|--exclusive) excl+=($2); shift 2 ;;
      -N|--min-args) minargs=$2; shift 2 ;;
      -X|--max-args) maxargs=$2; shift 2 ;;
      --name=*) name=${1#*=}; shift ;;
      --exclusive=*) excl+=(${1#*=}); shift ;;
      --min-args=*) minargs=${1#*=}; shift ;;
      --max-args=*) maxargs=${1#*=}; shift ;;
      -i|--ignore-unknown) ign=1; shift ;;
      -u|--move-unknown) mu=1; shift ;;
      -s|--stop-nonopt) stop=1; shift ;;
      -h|--help) shift ;;
      --) shift; break ;;
      *) specs+=($1); shift ;;
    esac
  done
  for spec in $specs; do
    s=${spec%%!*} sfx= short= long= amp= intid=
    [[ $s == *'&' ]] && { amp=1; s=${s%&}; }
    case $s in
      *'=?') sfx=o; s=${s%=?} ;;
      *'=+') sfx=m; s=${s%=+} ;;
      *=) sfx=r; s=${s%=} ;;
    esac
    # `m#max`: any `-NUM` sets the flag (the value form, as with `=`)
    if [[ $s == *'#'* ]]; then s=${s/\#//}; sfx=r; intid=1; fi
    if [[ $s == *[^[:alnum:]_/-]* ]]; then
      c=${s#${s%%[^[:alnum:]_/-]*}}
      print -u2 -r -- "$name: Invalid option spec '$spec' at char '${c[1]}'"
      return 2
    fi
    skipshort=
    if [[ $s == */* ]]; then short=${s%%/*}; long=${s#*/}
    elif [[ ${s[2]-} == - && ${#s} > 2 ]]; then short=${s[1]}; long=${s[3,-1]}; skipshort=1   # `m-min`: --min, variable also _flag_m
    elif (( ${#s} > 1 )); then long=$s
    else short=$s; fi
    id=${long:-$short}
    ids+=($id)
    [[ -n $intid ]] && intidname=$id
    [[ -n $amp ]] && noopt[$id]=1
    [[ $spec == *'!'* ]] && valid[$id]=${spec#*!}
    kind[$id]=$sfx
    [[ -n $short && -z $skipshort ]] && alias[-$short]=$id
    [[ -n $long ]] && alias[--$long]=$id
    alias[ids:$id]="$short $long"
    for a in $short $long; do unset "_flag_${a//-/_}"; done
  done
  while (( $# )); do
    a=$1; shift
    if [[ $a == -- ]]; then rest+=("$@"); break; fi
    if [[ -n $intidname && $a == -<-> ]]; then acc[$intidname]=${a#-}; opts+=($a); continue; fi
    if [[ $a == --?* ]]; then
      o=${a%%=*} val=
      [[ $a == *=* ]] && val=${a#*=}
      k=$alias[$o] consumed=
      if [[ -z $k ]]; then
        [[ -n $mu ]] && { opts+=($a); continue }
        [[ -n $ign ]] && { rest+=($a); continue }
        print -u2 -r -- "$name: $o: unknown option"; return 2
      fi
      if [[ -n $kind[$k] ]]; then
        if [[ $a != *=* ]]; then
          if [[ $kind[$k] == o ]]; then val=$nov
          elif (( $# )); then val=$1; consumed=1; shift
          else print -u2 -r -- "$name: $o: option requires an argument"; return 2; fi
        fi
        if [[ -n $valid[$k] ]]; then
          _flag_name=${o#--} _flag_value=$val _argparse_cmd=$name
          builtin eval "$valid[$k]" || return 1
        fi
        acc[$k]+="${acc[$k]+$us}$val"
      else
        acc[$k]+="${acc[$k]+$us}$o"
      fi
      if [[ -z $noopt[$k] ]]; then opts+=($a); [[ -n $consumed ]] && opts+=($val); fi
    elif [[ $a == -?* ]]; then
      i=2
      while (( i <= ${#a} )); do
        c=${a[i]}
        k=$alias[-$c]
        if [[ -z $k ]]; then
          [[ -n $mu ]] && { opts+=($a); break }
          [[ -n $ign ]] && { rest+=($a); break }
          print -u2 -r -- "$name: -$c: unknown option"; return 2
        fi
        if [[ -n $kind[$k] ]]; then
          val=${a[i+1,-1]} consumed=
          if [[ -z $val ]]; then
            if [[ $kind[$k] == o ]]; then val=$nov
            elif (( $# )); then val=$1; consumed=1; shift
            else print -u2 -r -- "$name: -$c: option requires an argument"; return 2; fi
          fi
          if [[ -n $valid[$k] ]]; then
            _flag_name=$c _flag_value=$val _argparse_cmd=$name
            builtin eval "$valid[$k]" || return 1
          fi
          acc[$k]+="${acc[$k]+$us}$val"
          if [[ -z $noopt[$k] ]]; then opts+=(-$c); [[ -n $consumed ]] && opts+=($val); fi
          break
        fi
        acc[$k]+="${acc[$k]+$us}-$c"
        [[ -z $noopt[$k] ]] && opts+=(-$c)
        (( i++ ))
      done
    else
      if [[ -n $stop ]]; then rest+=($a "$@"); break; fi
      rest+=($a)
    fi
  done
  for xs in $excl; do
    present=()
    for p in ${(s:,:)xs}; do
      k=$alias[-$p]; [[ -n $k ]] || k=$alias[--$p]; [[ -n $k ]] || k=$p
      [[ -n ${acc[$k]+x} ]] && present+=($k)
    done
    present=(${(ou)present})
    if (( ${#present} > 1 )); then
      for xa in $present; do xb+="${xb:+ }${(j:/:)${=alias[ids:$xa]}}"; done
      print -u2 -r -- "$name: $xb: options cannot be used together"; return 2
    fi
  done
  if [[ -n $minargs ]] && (( ${#rest} < minargs )); then
    print -u2 -r -- "$name: expected >= $minargs arguments; got ${#rest}"; return 1
  fi
  if [[ -n $maxargs ]] && (( ${#rest} > maxargs )); then
    print -u2 -r -- "$name: expected <= $maxargs arguments; got ${#rest}"; return 1
  fi
  for k in $ids; do
    [[ -n ${acc[$k]+x} ]] || continue
    vals=("${(@ps:\x1f:)acc[$k]}")
    vals=("${(@)vals:#$nov}")   # an optional value that was not given leaves the list empty
    for a in ${=alias[ids:$k]}; do
      a=${a//-/_}
      typeset -ga _flag_$a
      __fish_argflags+=(_flag_$a)
      builtin eval "_flag_$a=(\"\${(@)vals}\")"
    done
  done
  typeset -ga __fish_argv __fish_argv_opts
  __fish_argv=("${(@)rest}")
  __fish_argv_opts=("${(@)opts}")
  return 0
}
fish_vi_key_bindings() { bindkey -v; return 0 }
fish_default_key_bindings() { bindkey -e; return 0 }
fish_hybrid_key_bindings() { bindkey -e; return 0 }
fish_config() { return 0 }
# `complete` reached indirectly (`set c complete …; $c …`); a literal `complete`
# word is rewritten to __fish_complete by the translator
complete() { __fish_complete "$@" }
command_not_found_handler() {
  # fish autoloads a function from NAME.fish on $fish_function_path (and its own functions)
  if __fish_autoload_ok "$1"; then "$@"; return; fi
  print -u2 -r -- "fish: Unknown command: $1"; return 127
}
__fish_cl_words() { (( $# )) && print -rl -- "$@"; return 0 }
source() {
  # no file: fish reads the script from stdin (`cmd completion fish | source`)
  local __f=${1:--} __t
  (( $# )) && shift
  local __fish_srcfile=$__f
  __t=$('@ZSHRS@' --fish-translate --source -- "$__f") || return
  () { builtin eval "$__t" } "$@"
}
__fish_eval() {
  local __t
  __t=$(print -r -- "$*" | '@ZSHRS@' --fish-translate -) || return
  builtin eval "$__t"
}
eval() { __fish_eval "$@" }
__fish_uvar_save() {
  # $1 is the zsh parameter; an exported list lives in __fx_NAME
  local n=$1 name=$1 exp=
  local -a v
  case $n in __fx_*) name=${n#__fx_}; exp=--export ;; esac
  [[ ${(Pt)n} == *export* ]] && exp=--export
  v=("${(@P)n}")
  __fish_uvar_write $exp $name "${v[@]}"
  [[ -n ${__fish_uvar_marker-} ]] && : > $__fish_uvar_marker
  return 0
}
__fish_uvar_erase() {
  __fish_uvar_write -e ${1#__fx_}
  [[ -n ${__fish_uvar_marker-} ]] && : > $__fish_uvar_marker
  return 0
}
__fish_uvar_load
if [[ -o interactive ]]; then
  __fish_uvar_marker=${TMPDIR:-/tmp}/.fish_uvar.$$
  : > $__fish_uvar_marker
  __fish_uvar_sync() {
    [[ $__fish_uvar_file -nt $__fish_uvar_marker ]] || return 0
    __fish_uvar_load
    : > $__fish_uvar_marker
  }
  __fish_uvar_cleanup() { command rm -f $__fish_uvar_marker }
  precmd_functions+=(__fish_uvar_sync)
  # fish_title, when the user defines one, sets the terminal title before each prompt
  __fish_title_hook() { (( $+functions[fish_title] )) && print -rn -- $'\e]0;'"$(fish_title)"$'\a'; return 0 }
  precmd_functions+=(__fish_title_hook)
  zshexit_functions+=(__fish_uvar_cleanup)
fi
umask=(${(l:4::0:)$(builtin umask)})
CMD_DURATION=(0)
fish_pid=($$)
fish_kill_signal=(0)
FISH_VERSION=4.9.3
version=(4.9.3)
fish_prompt() { print -rn -- "${USER}@${HOST%%.*} ${PWD/#$HOME/~}> " }
__fish_prompt_render() {
  setopt local_options extended_glob
  local o
  o=$("$1")
  o=${o//\%/%%}
  print -rn -- "${o//(#m)$'\e'\[[0-9;]#m/%{${MATCH}%\}}"
}
if [[ -o interactive ]]; then
  setopt prompt_subst
  PROMPT='$(__fish_prompt_render fish_prompt)'
  __fish_rprompt() { (( $+functions[fish_right_prompt] )) && __fish_prompt_render fish_right_prompt }
  RPROMPT='$(__fish_rprompt)'
  # fish completes out of the box: compsys, with fish's own definitions first.
  # -u: fish loads completions from every directory on its paths without an
  # ownership audit, so compaudit must not stop the shell at a [y/n] prompt
  # (Homebrew leaves /opt/homebrew/share/zsh group-writable)
  autoload -Uz compinit && compinit -u -d ${ZSHRS_HOME:-$HOME/.zshrs}/fish.zcompdump
  zstyle ':completion:*' completer _fish_complete _complete _ignored
  # fish's TAB completes; it never expands a variable or a wildcard in place
  bindkey -M emacs '^I' complete-word
  bindkey -M viins '^I' complete-word
  compdef _precommand and or not begin
fi
# the `__` names above are the shell's own; `set -n` lists none of them
typeset -gA __fish_internal
for __n in ${(k)parameters[(I)__*]}; do __fish_internal[$__n]=1; done
unset __n
