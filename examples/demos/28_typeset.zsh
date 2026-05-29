#!/usr/bin/env zshrs
# typeset / declare — integer, array, assoc, readonly, exported.

echo "── integer ──"
typeset -i num=42
echo "num=$num type=int"
(( num *= 10 ))
echo "num after *=10: $num"

echo "── auto-eval as arithmetic on assign ──"
typeset -i ev
ev="2 + 3 * 4"
echo "ev (auto-evaluated) = $ev"

echo "── array ──"
typeset -a arr
arr=(one two three)
echo "size=${#arr[@]} first=${arr[1]}"

echo "── associative array ──"
typeset -A assoc
assoc[k1]=v1
assoc[k2]=v2
echo "k1=${assoc[k1]} k2=${assoc[k2]}"

echo "── readonly ──"
typeset -r RO=immutable
echo "RO=$RO"
# Attempting to reassign would fail; we only show it's set.

echo "── int with explicit base ──"
typeset -i 16 hexval=255
echo "hexval (base 16): $hexval"

typeset -i 2 binval=10
echo "binval (base 2): $binval"
