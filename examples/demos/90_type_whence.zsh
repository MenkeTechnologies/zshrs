#!/usr/bin/env zshrs
# Command resolution — type, whence, which, command -v.
# Ported from Src/builtin.c bin_whence + Src/exec.c findcmd + iscom.

# Define a few candidates of different kinds.
my_alias_demo() { echo "fn body"; }
alias greet='echo hello'

echo "── type ──"
type echo
type cd
type my_alias_demo
type greet
type ls

echo "── whence (zsh's canonical) ──"
whence echo
whence cd
whence my_alias_demo
whence greet
whence ls
whence nonexistent_xyz

echo "── whence -v (verbose) ──"
whence -v echo
whence -v my_alias_demo
whence -v greet

echo "── whence -c (csh-style) ──"
whence -c echo
whence -c my_alias_demo

echo "── command -v (POSIX) ──"
command -v echo
command -v ls
command -v greet

echo "── which (alias-resolving) ──"
which echo
which my_alias_demo
which greet

echo "── distinguishing kinds in dispatch ──"
classify() {
    local cmd=$1
    if (( $+aliases[$cmd] )); then
        echo "$cmd: alias"
    elif (( $+functions[$cmd] )); then
        echo "$cmd: function"
    elif (( $+builtins[$cmd] )); then
        echo "$cmd: builtin"
    elif [[ -n $(command -v $cmd) ]]; then
        echo "$cmd: external"
    else
        echo "$cmd: unknown"
    fi
}
for c in echo cd ls greet my_alias_demo nonexistent_xyz; do
    classify $c
done
