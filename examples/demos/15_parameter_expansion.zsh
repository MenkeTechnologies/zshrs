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
