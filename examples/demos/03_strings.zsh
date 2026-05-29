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
