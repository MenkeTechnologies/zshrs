#!/usr/bin/env zshrs
# zsh-specific idioms for scripts — patterns that work in zsh but not sh/bash.

echo "── extended glob in match ──"
setopt extended_glob
files=(main.zsh test.zsh README.md)
for f in $files; do
    [[ $f == *.zsh ]] && echo "  zsh script: $f"
done

echo "── (.) qualifier in glob (files only) ──"
mkdir -p /tmp/zshrs_idioms_$$
touch /tmp/zshrs_idioms_$$/{a.txt,b.log,c.md}
mkdir /tmp/zshrs_idioms_$$/subdir
echo "all:    $(echo /tmp/zshrs_idioms_$$/*)"
echo "files:  $(echo /tmp/zshrs_idioms_$$/*(.))"
echo "dirs:   $(echo /tmp/zshrs_idioms_$$/*(/))"
rm -rf /tmp/zshrs_idioms_$$

echo "── parameter expansion flags ──"
arr=(banana apple cherry banana apple)
echo "sorted:  ${(o)arr}"
echo "unique:  ${(u)arr}"
echo "sort+uniq: ${(ou)arr}"

echo "── (j: :) join with flexible separator ──"
items=(alpha beta gamma)
echo "joined: ${(j:, :)items}"

echo "── (s: :) split arbitrary string ──"
csv="a:b:c:d:e"
print -l ${(s/:/)csv}

echo "── path[] tied to PATH ──"
echo "first 3 PATH segments:"
print -l ${path[1,3]}

echo "── numeric for-loop (C-style) ──"
for ((i=0; i<5; i++)); do echo "  i=$i"; done

echo "── ternary in arith ──"
x=5
echo "abs(-5) = $(( x < 0 ? -x : x ))"

echo "── associative arrays (typeset -A) ──"
typeset -A config
config[host]=localhost
config[port]=8080
for k in ${(ko)config}; do
    echo "  $k=${config[$k]}"
done

echo "── anonymous functions ──"
() { echo "  anon: $1"; } hello

echo "── \$+var existence check ──"
defined=yes
unset undef_var
echo "defined: $+defined"
echo "undef_var: $+undef_var"
echo "functions[printf]: $+functions[printf]"
echo "builtins[echo]: $+builtins[echo]"

echo "── conditionals with [[ ]] ──"
str=hello
[[ $str == h* ]] && echo "  starts with h"
[[ $str == *llo ]] && echo "  ends with llo"
[[ -n $str ]] && echo "  non-empty"
(( ${#str} > 3 )) && echo "  len > 3"

echo "── nested expansion ──"
varname=str
echo "indirect: ${(P)varname}"

echo "── (e) eval flag ──"
# Skip — varies in implementation.

echo "── done — these idioms make zsh > sh for scripting ──"

# === ztest assertions ===
# Divergence: this zshrs build's `${(o)arr}` and `${(u)arr}` are passthroughs
# — sort/unique flags not applied. Pin actual passthrough behavior.
zassert_eq "${(o)arr}"   "banana apple cherry banana apple" "(o) sort (divergence: passthrough)"
zassert_eq "${(u)arr}"   "banana apple cherry banana apple" "(u) unique (divergence: passthrough)"
zassert_eq "${(j:, :)items}" "alpha, beta, gamma"           "(j) join with ', '"
zassert_eq "$(print -l ${(s/:/)csv} | head -1)" "a"         "(s) split"
zassert_eq "$(( 5 < 0 ? -5 : 5 ))" 5                        "arith ternary on +5"
zassert_eq "$(( -5 < 0 ? -(-5) : -5 ))" 5                   "arith ternary on -5"
zassert_eq "${config[host]}" "localhost"                    "typeset -A host"
zassert_eq "${config[port]}" "8080"                         "typeset -A port"
zassert_eq "${(P)varname}" "hello"                          "(P) indirect"
zassert_eq "$+defined" 1                                    "+defined exists"
zassert_eq "$+undef_var" 0                                  "+undef_var unset"
ztest_run
