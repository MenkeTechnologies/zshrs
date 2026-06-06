#!/usr/bin/env zshrs
# Compound parameter defaults — :- := :+ :? and pattern-trim variants.
# Ported from zsh's Src/subst.c paramsubst (default/alt branch).

echo "── \${var:-default} ──"
unset x
echo "x:-fallback = ${x:-fallback}"
echo "after :- x = ${x:-still-unset}"

echo "── \${var:=assigned-once} ──"
unset y
echo "y:=initial = ${y:=initial}"
echo "now y = $y"
echo "y:=ignored = ${y:=ignored}"
echo "y stays   = $y"

echo "── \${var:+alt} (alt-if-set) ──"
unset z
echo "z:+ALT = ${z:+ALT}"
z=present
echo "z:+ALT (now) = ${z:+ALT}"

echo "── \${var:?msg} (error if unset) ──"
unset w
w_safe=${w:-default-instead-of-error}
echo "would-error: ${w_safe}"
w=actual
echo "set: ${w:?should not error}"

echo "── chained / nested defaults ──"
unset a b c
echo "nested: ${a:-${b:-${c:-final-fallback}}}"
b=middle
echo "with b set: ${a:-${b:-${c:-fallback}}}"

echo "── trim variants combined ──"
file="archive.tar.gz"
echo "trim shortest .* (root):    ${file%.*}"
echo "trim longest .* (full root): ${file%%.*}"
echo "trim shortest /:  ${file#*.}"
echo "trim longest /:   ${file##*.}"

echo "── default per arg type ──"
arr=()
echo "arr default for empty: ${arr:-empty-array-fallback}"

typeset -A m
echo "m[missing]:-x = ${m[missing]:-x}"
m[foo]=set
echo "m[foo]:-x     = ${m[foo]:-x}"

echo "── null-string handling (treat empty as unset) ──"
empty=""
echo "empty:-A    = ${empty:-A}      # :- treats empty as unset"
echo "empty-A     = ${empty-A}       # plain - keeps empty as 'set'"

# === ztest assertions ===
unset xx
zassert_eq "${xx:-fallback}"  "fallback"  ":- on unset"
zassert_eq "${xx:-fallback}"  "fallback"  ":- doesn't assign"
zassert_eq "${xx:-still-unset}" "still-unset" "xx remains unset after :-"
unset yy
zassert_eq "${yy:=initial}"   "initial"   ":= assigns and returns"
zassert_eq "$yy"              "initial"   ":= persisted assignment"
zassert_eq "${yy:=ignored}"   "initial"   ":= no-op when already set"
unset zz
zassert_eq "${zz:+ALT}"       ""          ":+ on unset returns empty"
zz=present
zassert_eq "${zz:+ALT}"       "ALT"       ":+ on set returns alt"
unset a b c
zassert_eq "${a:-${b:-${c:-final}}}"  "final"  "nested :- chain"
b=middle
zassert_eq "${a:-${b:-${c:-final}}}"  "middle" "nested :- picks first set"
file="archive.tar.gz"
zassert_eq "${file%.*}"  "archive.tar" "trim shortest .* (root)"
zassert_eq "${file%%.*}" "archive"     "trim longest .* (full root)"
zassert_eq "${file#*.}"  "tar.gz"      "trim shortest prefix .*"
zassert_eq "${file##*.}" "gz"          "trim longest prefix .*"
empty=""
zassert_eq "${empty:-A}" "A" ":- treats empty as unset"
zassert_eq "${empty-A}"  ""  "plain - keeps empty as set"
ztest_run

