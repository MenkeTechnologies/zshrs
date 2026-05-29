#!/usr/bin/env zshrs
# Matrix-style nested-loop output — multiplication table.

echo "── 9x9 multiplication table ──"
for ((i = 1; i <= 9; i++)); do
    for ((j = 1; j <= 9; j++)); do
        printf "%4d" $(( i * j ))
    done
    echo
done

echo
echo "── triangular ──"
for ((i = 1; i <= 7; i++)); do
    for ((j = 1; j <= i; j++)); do
        printf "* "
    done
    echo
done

echo
echo "── diamond ──"
for ((i = 1; i <= 5; i++)); do
    for ((j = i; j < 5; j++)); do printf " "; done
    for ((j = 1; j <= 2 * i - 1; j++)); do printf "*"; done
    echo
done
for ((i = 4; i >= 1; i--)); do
    for ((j = 5; j > i; j--)); do printf " "; done
    for ((j = 1; j <= 2 * i - 1; j++)); do printf "*"; done
    echo
done
