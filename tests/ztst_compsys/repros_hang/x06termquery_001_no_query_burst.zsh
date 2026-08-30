# Minimal hanging input for X06termquery#1 "foot response to terminal queries",
# the FIRST assertion in the file -- all 11 never complete.
#
#   <harness-zsh> -f x06termquery_001_no_query_burst.zsh <shell-under-test>
#
# zsh prints the query burst it sends on startup and exits 0:
#   ^[]11;?^[\ ^[]10;?^[\ ^[]12;?^[\ ^[P+q524742^[\ ^[[>0q ^[[c  ^M
# zshrs sends none of it, so upstream's `zpty -r zsh REPLY $'\e*\r'` -- the
# first thing X06termquery's own termresp() does -- blocks forever.
#
# A BLOCKED READ in the harness, caused by a missing capability in the shell
# under test.  Confirmed with ZSHRS_NATIVE_ZLE_FX=0 (the suite's setting) as
# well as with the overlays on; in both cases the first bytes zshrs writes are
# the echoed input line, never an escape query.
#
emulate -R zsh
setopt extendedglob
[[ -d Modules/zsh ]] && module_path=( $PWD/Modules )
zmodload zsh/zpty || exit 1
export PS1= PS2= COLORTERM= TERM=
typeset +x TERM_PROGRAM
zpty zsh "$1 -fiV +Z"
zpty -w zsh "module_path=( ${(j< >)${(@q-)module_path}} \$module_path )"
zpty -w zsh ".term.extensions=( -bracketed-paste -integration )"
zpty -w zsh "setopt zle"
print -u2 "waiting for a terminal query matching \$'\\e*\\r' ..."
zpty -r zsh REPLY $'\e*\r'
print -r -- "got: ${(V)REPLY}"
zpty -d
