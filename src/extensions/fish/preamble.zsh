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
__fish_slice() {
  local -a src=("${(@P)2}") r
  local n=$#src a=$3 b=$4 i
  (( a < 0 )) && a=$(( n + a + 1 ))
  (( b < 0 )) && b=$(( n + b + 1 ))
  if (( a >= 1 && b >= 1 && a <= n )); then
    (( b > n )) && b=n
    if (( a <= b )); then
      for (( i = a; i <= b; i++ )); do r+=("${src[i]}"); done
    else
      for (( i = a; i >= b; i-- )); do r+=("${src[i]}"); done
    fi
  fi
  typeset -ga $1
  builtin eval "$1=(\"\${(@)r}\")"
}
status() {
  case ${1-} in
    is-interactive|-i|--is-interactive) [[ -o interactive ]] ;;
    is-login|-l|--is-login) [[ -o login ]] ;;
    is-command-substitution|is-block|is-breakpoint|is-event|-c|-b|-n) return 1 ;;
    job-control|-j) return 1 ;;
    filename|-f|--current-filename) print -r -- ${ZSH_ARGZERO:-Standard input} ;;
    current-command) print -r -- ${funcstack[2]:-fish} ;;
    fish-path) print -r -- '@ZSHRS@' ;;
    current-line-number|-n) print -r -- $LINENO ;;
    stack-trace|print-stack-trace|test-feature|buildinfo|terminal-os) return 0 ;;
    *) [[ -o interactive ]] && print -r -- 'This is an interactive session' || print -r -- 'This is a non-interactive session' ;;
  esac
}
__fish_set_list() {
  local scope= names= exp= n k line v
  local -a vals out
  for k; do
    case $k in
      -g|--global|-U|--universal) scope=g ;;
      -l|--local) scope=l ;;
      -n|--names) names=1 ;;
      -x|--export) exp=1 ;;
    esac
  done
  for n in ${(ok)parameters}; do
    k=$n
    case $n in
      _fl_top_*) k=${n#_fl_top_}; [[ $scope == g ]] && continue ;;
      _fl<->_*) k=${n#_fl<->_}; [[ $scope == g ]] && continue ;;
      __fish*|_fish_x|_fl*) continue ;;
      _fish_v_*) k=${n#_fish_v_}; [[ $scope == l ]] && continue ;;
      path) k=PATH; [[ $scope == l ]] && continue ;;
      cdpath) k=CDPATH; [[ $scope == l ]] && continue ;;
      manpath) k=MANPATH; [[ $scope == l ]] && continue ;;
      *) [[ $scope == l ]] && continue ;;
    esac
    [[ ${(Pt)n} == *association* || ${(Pt)n} == *special* && ${(Pt)n} != *export* ]] && continue
    [[ -n $exp && ${(Pt)n} != *export* ]] && continue
    if [[ -n $names ]]; then out+=($k); continue; fi
    if [[ ${(Pt)n} == array* ]]; then vals=("${(@P)n}"); else vals=("${(P)n}"); fi
    line=$k
    if (( ${#vals} == 1 )); then
      [[ -n ${vals[1]} ]] && line+=" ${(q-)vals[1]}"
    else
      for v in "${vals[@]}"; do line+=" '${v//\'/\\\'}'"; done
    fi
    out+=($line)
  done
  (( ${#out} )) && print -rl -- "${(@)out}"
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
    if [[ $n == *PATH ]]; then vals=("${(@s.:.)${(P)z}}"); else vals=(${=${(P)z}}); fi
  else vals=("${(P)z}"); fi
  print -r -- "\$${n}: set in $kind scope, $exp, with ${#vals} elements"
  local i
  for (( i = 1; i <= ${#vals}; i++ )); do print -r -- "\$${n}[$i]: |${vals[i]}|"; done
  return 0
}
typeset -gA __fish_abbrs
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
  case $n in
    PATH) n=path ;; CDPATH) n=cdpath ;; MANPATH) n=manpath ;; hostname) n=HOST ;;
    path|cdpath|fpath|manpath|mailpath|module_path|signals|options|commands|functions|aliases|parameters|history|dirstack|widgets|keymaps|modules|nameddirs|userdirs|galiases|saliases|funcstack|jobtexts|builtins|reswords|status|prompt|psvar|watch) n=_fish_v_$n ;;
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
  local q= t= a n k r=0
  for a; do
    case $a in
      -q|--quiet) q=1 ;;
      -t|--type) t=1 ;;
      -*) ;;
      *) n=$a
         k=$(whence -w -- $n 2>/dev/null); k=${k##*: }
         [[ -n $k && $k != none ]] || { r=1; [[ -n $q ]] || print -u2 -r -- "type: Could not find '$n'"; continue }
         [[ -n $q ]] && continue
         case $k in
           function) [[ -n $t ]] && print -r -- function || print -r -- "$n is a function" ;;
           builtin) [[ -n $t ]] && print -r -- builtin || print -r -- "$n is a builtin" ;;
           command) [[ -n $t ]] && print -r -- file || print -r -- "$n is ${$(whence -p -- $n)}" ;;
           *) [[ -n $t ]] && print -r -- $k || print -r -- "$n is a $k" ;;
         esac ;;
    esac
  done
  return $r
}
functions() {
  local a n=() q= e= listnames= alln=
  for a; do
    case $a in
      -q|--query) q=1 ;;
      -e|--erase) e=1 ;;
      -n|--names) listnames=1 ;;
      -a|--all) alln=1 ;;
      -*) ;;
      *) n+=($a) ;;
    esac
  done
  if [[ -n $listnames ]]; then
    if [[ $(whence -w __fish_function_names 2>/dev/null) != *none* && -n $(whence -w __fish_function_names 2>/dev/null) ]]; then __fish_function_names ${alln:+-a}; else print -rl -- ${(ok)functions}; fi
    return
  fi
  if [[ -n $q ]]; then (( ${#n} )) && (( $+functions[$n[1]] )); return; fi
  if [[ -n $e ]]; then for a in $n; do unfunction -- $a 2>/dev/null; done; return 0; fi
  if (( ! ${#n} )); then print -rl -- ${(ko)functions}; return 0; fi
  for a in $n; do
    (( $+functions[$a] )) || { print -u2 -r -- "functions: Unknown function '$a'"; return 1 }
    builtin functions -- $a
  done
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
  dirprev+=($OLDPWD)
  dirnext=()
  (( ${#dirprev} > 25 )) && dirprev[1]=()
}
chpwd_functions+=(__fish_chpwd)
prevd() {
  local n=1 d
  [[ ${1-} == -l ]] && shift
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
}
nextd() {
  local n=1 d
  [[ ${1-} == -l ]] && shift
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
  up-or-search up-line-or-search down-or-search down-line-or-search complete expand-or-complete
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
typeset -gA __fish_events
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
__fish_argparse() {
  local name=argparse ign= stop= mu= amp= consumed= skipshort= spec s sfx short long id a o val k i c us=$'\x1f'
  local -a vals opts=()
  local -A kind=() alias=() acc=() noopt=() valid=()
  local -a specs=() rest=() ids=()
  while (( $# )); do
    case $1 in
      -n|--name) name=$2; shift 2 ;;
      -x|--exclusive|-N|--min-args|-X|--max-args) shift 2 ;;
      --name=*) name=${1#*=}; shift ;;
      --exclusive=*|--min-args=*|--max-args=*) shift ;;
      -i|--ignore-unknown) ign=1; shift ;;
      -u|--move-unknown) mu=1; shift ;;
      -s|--stop-nonopt) stop=1; shift ;;
      -h|--help) shift ;;
      --) shift; break ;;
      *) specs+=($1); shift ;;
    esac
  done
  for spec in $specs; do
    s=${spec%%!*} sfx= short= long= amp=
    [[ $s == *'&' ]] && { amp=1; s=${s%&}; }
    case $s in
      *'=?') sfx=o; s=${s%=?} ;;
      *'=+') sfx=m; s=${s%=+} ;;
      *=) sfx=r; s=${s%=} ;;
    esac
    skipshort=
    if [[ $s == */* ]]; then short=${s%%/*}; long=${s#*/}
    elif [[ ${s[2]-} == - && ${#s} > 2 ]]; then short=${s[1]}; long=${s[3,-1]}; skipshort=1   # `m-min`: --min, variable also _flag_m
    elif (( ${#s} > 1 )); then long=$s
    else short=$s; fi
    id=${long:-$short}
    ids+=($id)
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
    if [[ $a == --?* ]]; then
      o=${a%%=*} val=
      [[ $a == *=* ]] && val=${a#*=}
      k=$alias[$o] consumed=
      if [[ -z $k ]]; then
        [[ -n $mu ]] && { opts+=($a); continue }
        [[ -n $ign ]] && { rest+=($a); continue }
        print -u2 -r -- "$name: Unknown option '$o'"; return 2
      fi
      if [[ -n $kind[$k] ]]; then
        if [[ $a != *=* ]]; then
          if [[ $kind[$k] == o ]]; then val=
          elif (( $# )); then val=$1; consumed=1; shift
          else print -u2 -r -- "$name: Expected argument for option $o"; return 2; fi
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
          print -u2 -r -- "$name: Unknown option '-$c'"; return 2
        fi
        if [[ -n $kind[$k] ]]; then
          val=${a[i+1,-1]} consumed=
          if [[ -z $val ]]; then
            if [[ $kind[$k] == o ]]; then val=
            elif (( $# )); then val=$1; consumed=1; shift
            else print -u2 -r -- "$name: Expected argument for option -$c"; return 2; fi
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
  for k in $ids; do
    [[ -n ${acc[$k]+x} ]] || continue
    vals=("${(@ps:\x1f:)acc[$k]}")
    for a in ${=alias[ids:$k]}; do
      a=${a//-/_}
      typeset -ga _flag_$a
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
  if __fish_autoload "$1"; then "$@"; return; fi
  print -u2 -r -- "fish: Unknown command: $1"; return 127
}
__fish_cl_words() { (( $# )) && print -rl -- "$@"; return 0 }
source() {
  # no file: fish reads the script from stdin (`cmd completion fish | source`)
  local __f=${1:--} __t
  (( $# )) && shift
  __t=$('@ZSHRS@' --fish-translate --source -- "$__f") || return
  () { builtin eval "$__t" } "$@"
}
__fish_eval() {
  local __t
  __t=$(print -r -- "$*" | '@ZSHRS@' --fish-translate -) || return
  builtin eval "$__t"
}
eval() { __fish_eval "$@" }
__fish_uvar_file=${HOME}/.zshrs/fish_universal.zsh
__fish_uvar_save() {
  local n=$1 f=$__fish_uvar_file line
  local -a keep v
  [[ -r $f ]] && keep=("${(@f)$(<$f)}")
  keep=("${(@)keep:#(typeset -gx |)${n}=*}")
  if [[ ${(Pt)n} == *export* ]]; then
    line="typeset -gx $n=${(q)${(P)n}}"
  else
    v=("${(@P)n}")
    line="$n=(${(j: :)${(@q)v}})"
  fi
  [[ -d ${f:h} ]] || mkdir -p ${f:h}
  print -rl -- "${(@)keep:#}" $line > $f
}
__fish_uvar_erase() {
  local n=$1 f=$__fish_uvar_file
  local -a keep
  [[ -r $f ]] || return 0
  keep=("${(@f)$(<$f)}")
  keep=("${(@)keep:#(typeset -gx |)${n}=*}")
  print -rl -- "${(@)keep:#}" > $f
}
[[ -r $__fish_uvar_file ]] && builtin source $__fish_uvar_file
if [[ -o interactive ]]; then
  __fish_uvar_marker=${TMPDIR:-/tmp}/.fish_uvar.$$
  : > $__fish_uvar_marker
  __fish_uvar_sync() {
    [[ $__fish_uvar_file -nt $__fish_uvar_marker ]] || return 0
    builtin source $__fish_uvar_file
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
  # fish completes out of the box: compsys, with fish's own definitions first
  autoload -Uz compinit && compinit -d ${ZSHRS_HOME:-$HOME/.zshrs}/fish.zcompdump
  zstyle ':completion:*' completer _fish_complete _complete _ignored
  compdef _precommand and or not begin
fi
