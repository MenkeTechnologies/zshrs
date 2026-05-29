#!/usr/bin/env zshrs
# Perfect / abundant / deficient classification.

sum_proper_divisors() {
    local n=$1 s=0 i
    for ((i = 1; i*i <= n; i++)); do
        if (( n % i == 0 )); then
            (( s += i ))
            local q=$(( n / i ))
            if (( q != i && q != n )); then
                (( s += q ))
            fi
        fi
    done
    echo $s
}

classify() {
    local n=$1
    local sd=$(sum_proper_divisors $n)
    if (( sd == n )); then echo perfect
    elif (( sd > n )); then echo abundant
    else echo deficient
    fi
}

echo "── classify 1..30 ──"
for ((n = 1; n <= 30; n++)); do
    printf "%3d: %s\n" $n "$(classify $n)"
done

echo "── perfect numbers ≤ 100 ──"
for ((n = 1; n <= 100; n++)); do
    [[ "$(classify $n)" == perfect ]] && echo "  $n"
done

echo "── count abundant in 1..100 ──"
ab=0
for ((n = 1; n <= 100; n++)); do
    [[ "$(classify $n)" == abundant ]] && (( ab++ ))
done
echo "abundant count: $ab"
