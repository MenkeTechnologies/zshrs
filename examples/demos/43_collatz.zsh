#!/usr/bin/env zshrs
# Collatz (3n+1) — print the trajectory + length.

collatz_seq() {
    local n=$1
    local seq=($n)
    while (( n != 1 )); do
        if (( n % 2 == 0 )); then
            (( n /= 2 ))
        else
            (( n = 3*n + 1 ))
        fi
        seq+=($n)
    done
    echo "${seq[@]}"
}

collatz_len() {
    local n=$1 c=0
    while (( n != 1 )); do
        if (( n % 2 == 0 )); then
            (( n /= 2 ))
        else
            (( n = 3*n + 1 ))
        fi
        (( c++ ))
    done
    echo $c
}

echo "── n=6 sequence ──"
collatz_seq 6
echo "── n=27 sequence (length only) ──"
echo "len = $(collatz_len 27)"

echo "── lengths for 1..15 ──"
for n in {1..15}; do
    printf "%2d → %d\n" $n $(collatz_len $n)
done

echo "── longest length for 1..100 ──"
best=1 bestlen=0
for ((n = 1; n <= 100; n++)); do
    l=$(collatz_len $n)
    if (( l > bestlen )); then
        bestlen=$l
        best=$n
    fi
done
echo "n=$best gives length $bestlen"
