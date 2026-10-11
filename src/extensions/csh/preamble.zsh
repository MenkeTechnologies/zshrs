command_not_found_handler() { print -u2 -r -- "$1: Command not found."; return 1 }
_csh_die() { print -u2 -r -- "$1"; [[ -o interactive ]] || exit 1; return 1 }
_csh_direrr() {
  local m
  if [[ -e $1 && ! -d $1 ]]; then m='Not a directory.'
  elif [[ -d $1 ]]; then m='Permission denied.'
  else m='No such file or directory.'; fi
  _csh_die "$1: $m"
}
cd() {
  local _t=${1-$HOME} _p=
  (( $# > 1 )) && { _csh_die 'cd: Too many arguments.'; return }
  [[ $_t == - ]] && _t=$OLDPWD
  [[ ${1-} == (/|./|../|~|-)* || -d $_t || $# == 0 ]] || _p=1
  if [[ -n $_t ]] && builtin cd -- "$_t" >/dev/null 2>&1; then
    [[ -z $_p ]] || print -r -- "$(dirs) "
    return 0
  fi
  _csh_direrr "$_t"
}
pushd() {
  local -a _a=(${@:#-q})
  if (( $#_a == 1 )) && [[ $_a[1] != [+-]<-> && ! -d $_a[1] ]]; then
    _csh_direrr $_a[1]; return
  fi
  builtin pushd "$@"
}
popd() {
  local -a _a=(${@:#-q})
  if (( ! $#_a && ! $#dirstack )); then _csh_die 'popd: Directory stack empty.'; return; fi
  builtin popd "$@"
}
printenv() {
  if (( ! $# )); then command env; return; fi
  [[ ${parameters[$1]-} == *export* ]] || return 1
  print -r -- ${(P)1}
}
unset OLDPWD
typeset -gA _csh_alias _csh_alias_ls
# `time cmd` / `time` report, tcsh's default $time format
# `%Uu %Ss %E %P %X+%Dk %I+%Oio %Fpf+%Ww`, on STDOUT (zsh's goes to stderr).
# Reads one line `user sys elapsed cpu% pagefaults swaps` — seconds as plain
# numbers, `cpu%` with or without the `%`.
_csh_time_fmt() {
  local u s e p f w
  local -i m
  read -r u s e p f w
  p=${p%\%}
  m=$(( ${e%%.*} / 60 ))
  printf '%.3fu %.3fs %d:%05.2f %.1f%%\t0+0k 0+0io %dpf+%dw\n' \
    $u $s $m $(( e - m * 60 )) ${p:-0} ${f:-0} ${w:-0}
}
# bare `time`: the shell's own CPU time (from `times`) and wall time.
_csh_time_self() {
  local l=${${(f)"$(times)"}[1]} a b
  a=${l%% *} b=${l##* }
  local -F u s
  u=$(( ${a%%m*} * 60 + ${${a#*m}%s} ))
  s=$(( ${b%%m*} * 60 + ${${b#*m}%s} ))
  print -r -- "$u $s $SECONDS 0 0 0" | _csh_time_fmt
}
