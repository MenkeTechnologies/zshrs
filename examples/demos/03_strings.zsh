#!/usr/bin/env zshrs
# String operations — length, case, slice, concat, search.
s="Hello, World!"

echo "── basics ──"
echo "s='$s'"
echo "len=${#s}"

echo "── case conversion ──"
echo "upper: ${s:u}"
echo "lower: ${s:l}"

echo "── substring (1-based) ──"
echo "first 5: ${s[1,5]}"
echo "chars 8-12: ${s[8,12]}"
echo "last 6: ${s[-6,-1]}"

echo "── prefix/suffix ──"
echo "trim suffix: ${s%, *}"
echo "trim prefix: ${s#*, }"

echo "── replace ──"
echo "first /l/L: ${s/l/L}"
echo "all /l/L: ${s//l/L}"

echo "── concat ──"
prefix=">>>"
suffix="<<<"
echo "${prefix} $s ${suffix}"

# === ztest assertions ===
zassert_eq "${#s}"    13              "len"
zassert_eq "${s:u}"   "HELLO, WORLD!" "upper"
zassert_eq "${s:l}"   "hello, world!" "lower"
zassert_eq "${s[1,5]}" "Hello"        "slice 1..5"
zassert_eq "${s[8,12]}" "World"       "slice 8..12"
zassert_eq "${s[-6,-1]}" "World!"     "negative slice"
zassert_eq "${s%, *}" "Hello"         "strip suffix"
zassert_eq "${s#*, }" "World!"        "strip prefix"
zassert_eq "${s/l/L}" "HeLlo, World!" "replace first"
zassert_eq "${s//l/L}" "HeLLo, WorLd!" "replace all"
zassert_eq "${prefix} $s ${suffix}" ">>> Hello, World! <<<" "concat"
ztest_run
