#!/usr/bin/env zshrs
# Control flow — if/elif/else, case, while, until.
n=15

echo "── if/elif/else ──"
if (( n > 100 )); then
    echo "big"
elif (( n > 10 )); then
    echo "medium"
elif (( n > 0 )); then
    echo "small"
else
    echo "zero or negative"
fi

echo "── case ──"
case $n in
    0) echo "zero" ;;
    [1-9]) echo "single digit" ;;
    1[0-9]) echo "teens" ;;
    *) echo "big" ;;
esac

echo "── while ──"
i=0
while (( i < 3 )); do
    echo "while i=$i"
    (( i++ ))
done

echo "── until ──"
i=0
until (( i >= 3 )); do
    echo "until i=$i"
    (( i++ ))
done

echo "── break/continue ──"
for i in {1..6}; do
    (( i == 4 )) && continue
    (( i == 6 )) && break
    echo "loop i=$i"
done
