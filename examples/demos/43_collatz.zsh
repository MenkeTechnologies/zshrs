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

# === ztest assertions ===
zassert_eq "$(collatz_seq 6)"   "6 3 10 5 16 8 4 2 1" "collatz sequence n=6"
zassert_eq "$(collatz_seq 1)"   "1"                    "collatz n=1 base case"
zassert_eq "$(collatz_len 1)"   0                      "collatz length n=1"
zassert_eq "$(collatz_len 6)"   8                      "collatz length n=6"
zassert_eq "$(collatz_len 27)"  111                    "collatz length n=27 (famous)"
zassert_eq "$(collatz_len 7)"   16                     "collatz length n=7"
zassert_eq "$best"              97                     "max-length n in 1..100"
zassert_eq "$bestlen"           118                    "max length value in 1..100"
ztest_run
