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
__fish_slice() {
  local -a src=("${(@P)2}") r
  local n=$#src a=$3 b=$4 i
  (( a < 0 )) && a=$(( n + a + 1 ))
  (( b < 0 )) && b=$(( n + b + 1 ))
  if (( a <= b )); then
    for (( i = a; i <= b; i++ )); do (( i >= 1 && i <= n )) && r+=("${src[i]}"); done
  else
    for (( i = a; i >= b; i-- )); do (( i >= 1 && i <= n )) && r+=("${src[i]}"); done
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
__fish_set_dynamic() {
  local a
  while [[ ${1-} == -* ]]; do shift; done
  a=$1; shift
  builtin eval "typeset -g -a $a; $a=(\"\$@\")"
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
  local a n=() q= e=
  for a; do
    case $a in
      -q|--query) q=1 ;;
      -e|--erase) e=1 ;;
      -n|--names|-a|--all) ;;
      -*) ;;
      *) n+=($a) ;;
    esac
  done
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
      -c|--print-colors|-v|--version) ;;
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
    clear|delete) return 0 ;;
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
  local a t ch tok= cut= cur= buf=
  while (( $# )); do
    a=$1; shift
    case $a in
      -f|--function) t=${__fish_widgets[$1]:-$1}; shift; zle $t 2>/dev/null ;;
      -r|--replace) BUFFER=${1-}; CURSOR=${#BUFFER}; shift $(( $# > 0 )) ;;
      -i|--insert) LBUFFER+=${1-}; shift $(( $# > 0 )) ;;
      -a|--append) RBUFFER+=${1-}; shift $(( $# > 0 )) ;;
      -C|--cursor) if (( $# )); then CURSOR=$1; shift; else print -r -- $CURSOR; return 0; fi ;;
      -b|--current-buffer) buf=1 ;;
      -t|--current-token) cur=1 ;;
      -o|--tokenize) tok=1 ;;
      -c|--cut-at-cursor) cut=1 ;;
      -[a-z][a-z]*)
        for ch in ${(s::)a#-}; do
          case $ch in o) tok=1 ;; c) cut=1 ;; t) cur=1 ;; b) buf=1 ;; esac
        done ;;
    esac
  done
  if [[ -n $tok ]]; then
    if (( ${+__fish_toks} )); then print -rl -- "${(@)__fish_toks}"; else print -rl -- ${(z)LBUFFER}; fi
  elif [[ -n $cur ]]; then
    if (( ${+__fish_cur} )); then print -r -- $__fish_cur; else t=(${(z)LBUFFER}); print -r -- ${t[-1]-}; fi
  elif [[ -n $buf ]]; then
    print -r -- $BUFFER
  fi
  return 0
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
  local name=argparse ign= stop= mu= amp= consumed= spec s sfx short long id a o val k i c us=$'\x1f'
  local -a vals opts=()
  local -A kind=() alias=() acc=() noopt=()
  local -a specs=() rest=() ids=()
  while (( $# )); do
    case $1 in
      -n|--name) name=$2; shift 2 ;;
      -x|--exclusive|-N|--min-args|-X|--max-args) shift 2 ;;
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
    if [[ $s == */* ]]; then short=${s%%/*}; long=${s#*/}
    elif (( ${#s} > 1 )); then long=$s
    else short=$s; fi
    id=${long:-$short}
    ids+=($id)
    [[ -n $amp ]] && noopt[$id]=1
    kind[$id]=$sfx
    [[ -n $short ]] && alias[-$short]=$id
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
fish_vi_key_bindings() { return 0 }
fish_default_key_bindings() { return 0 }
fish_config() { return 0 }
command_not_found_handler() { print -u2 -r -- "fish: Unknown command: $1"; return 127 }
source() {
  local __f=$1 __t
  shift
  __t=$('@ZSHRS@' --fish-translate -- "$__f") || return
  () { builtin eval "$__t" } "$@"
}
eval() {
  local __t
  __t=$(print -r -- "$*" | '@ZSHRS@' --fish-translate -) || return
  builtin eval "$__t"
}
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
  zshexit_functions+=(__fish_uvar_cleanup)
fi
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
fi
