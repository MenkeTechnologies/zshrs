#!/usr/bin/env zshrs
# Variable assignment forms — scalar, array, assoc, +=, =, declare-and-assign.
# Ported from Src/exec.c addvars + Src/builtin.c bin_typeset.

echo "── plain scalar ──"
a=hello
echo "a=$a"

echo "── reassign ──"
a=world
echo "a=$a"

echo "── append with += ──"
s=foo
s+=bar
echo "s=$s"

echo "── empty assignment (unset → empty) ──"
b=
echo "b='$b' len=${#b}"

echo "── multiple on one line ──"
x=1 y=2 z=3
echo "x=$x y=$y z=$z"

echo "── env-prefix (var visible only to one command) ──"
DEMO_X=42 env | grep DEMO_X | head -1
echo "after: DEMO_X=${DEMO_X:-unset_globally}"

echo "── array literal ──"
arr=(alpha beta gamma)
echo "arr count=${#arr[@]} arr[1]=${arr[1]}"

echo "── array append += ──"
arr+=(delta)
echo "after +=: count=${#arr[@]} last=${arr[-1]}"

echo "── array reassignment ──"
arr2=(a b c)
arr2=(x y z)
echo "arr2: ${arr2[@]}"

echo "── single-element replace ──"
arr3=(red green blue)
arr3[2]=YELLOW
echo "after [2]=YELLOW: ${arr3[@]}"

echo "── assoc literal (typeset -A required) ──"
typeset -A map=(k1 v1 k2 v2 k3 v3)
echo "size=${#map[@]} k2=${map[k2]}"

echo "── assoc single-key ──"
map[k4]=v4
echo "after [k4]=v4: size=${#map[@]}"

echo "── declare-and-assign forms ──"
typeset -i n=42
typeset -a a2=(x y z)
typeset -A m2=(p 1 q 2)
typeset -r RO=42
echo "n=$n type=${(t)n}"
echo "a2=${a2[@]} type=${(t)a2}"
echo "m2[p]=${m2[p]} type=${(t)m2}"
echo "RO=$RO type=${(t)RO}"

echo "── unset forms ──"
unset a b s n
echo "after unset: a='${a:-cleared}' b='${b:-cleared}'"
