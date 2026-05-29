#!/usr/bin/env zshrs
# For-loop variants — list, range, c-style, repeat.
echo "── for over list ──"
for x in alpha beta gamma; do
    echo "list: $x"
done

echo "── for over brace range ──"
for i in {1..5}; do
    echo "brace: $i"
done

echo "── for over letter range ──"
for c in {a..e}; do
    echo "letter: $c"
done

echo "── for over stepped range ──"
for i in {0..10..2}; do
    echo "step: $i"
done

echo "── c-style for ──"
for ((i = 0; i < 3; i++)); do
    echo "cstyle: i=$i"
done

echo "── c-style nested ──"
for ((i = 1; i <= 3; i++)); do
    for ((j = 1; j <= 3; j++)); do
        printf "(%d,%d) " $i $j
    done
    echo
done

echo "── repeat N ──"
repeat 4 echo "knock"
