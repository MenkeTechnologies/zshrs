#!/usr/bin/env zshrs
# Parameter expansion — default, length, substring, replace.

echo "── default value ──"
unset undef
echo "undef: ${undef:-fallback}"
echo "undef (still): ${undef:-fallback}"

echo "── assign default ──"
unset undef2
echo "first: ${undef2:=assigned}"
echo "second: $undef2"

echo "── length ──"
s="abcdefgh"
echo "length of '$s' = ${#s}"

echo "── substring ──"
echo "s[1,3]   = ${s[1,3]}"
echo "s[4,-1]  = ${s[4,-1]}"
echo "s[-3,-1] = ${s[-3,-1]}"

echo "── prefix removal ──"
path=/usr/local/bin/zshrs
echo "after first / : ${path#*/}"
echo "after last /  : ${path##*/}"

echo "── suffix removal ──"
file="readme.md.bak"
echo "trim shortest .* : ${file%.*}"
echo "trim longest .*  : ${file%%.*}"

echo "── replace ──"
str="banana"
echo "first a → A: ${str/a/A}"
echo "all   a → A: ${str//a/A}"

# === ztest assertions ===
unset uvar
zassert_eq "${uvar:-fallback}" "fallback"  "default fallback for unset"
unset uvar2
: "${uvar2:=hello}"
zassert_eq "$uvar2"            "hello"     ":= assigns and sets"
zassert_eq "${#s}"             "8"         "length"
zassert_eq "${s[1,3]}"         "abc"       "substring [1,3]"
zassert_eq "${s[4,-1]}"        "defgh"     "substring [4,-1]"
zassert_eq "${s[-3,-1]}"       "fgh"       "negative substring"
zassert_eq "${path#*/}"        "usr/local/bin/zshrs" "# strips shortest prefix"
zassert_eq "${path##*/}"       "zshrs"     "## strips longest prefix"
zassert_eq "${file%.*}"        "readme.md" "% strips shortest suffix"
zassert_eq "${file%%.*}"       "readme"    "%% strips longest suffix"
zassert_eq "${str/a/A}"        "bAnana"    "/ replaces first match"
zassert_eq "${str//a/A}"       "bAnAnA"    "// replaces all matches"
ztest_run
