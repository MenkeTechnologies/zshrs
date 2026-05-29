#!/usr/bin/env zshrs
# Associative arrays — key/value storage and iteration.
typeset -A colors
colors[red]=255
colors[green]=128
colors[blue]=64
colors[white]=255

echo "── lookup ──"
echo "red=${colors[red]}"
echo "green=${colors[green]}"
echo "blue=${colors[blue]}"

echo "── keys ──"
for k in ${(k)colors}; do
    echo "key=$k"
done | sort

echo "── values ──"
for v in ${(v)colors}; do
    echo "val=$v"
done | sort

echo "── k/v pairs ──"
for k v in "${(@kv)colors}"; do
    echo "$k -> $v"
done | sort

echo "── inline literal ──"
typeset -A scores=( alice 90 bob 85 carol 92 )
echo "alice=${scores[alice]}"
echo "bob=${scores[bob]}"
echo "carol=${scores[carol]}"
echo "size=${#scores[@]}"
