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

# === ztest assertions ===
zassert_eq "$(sum_proper_divisors 6)"   6    "sum-divisors 6 (1+2+3)"
zassert_eq "$(sum_proper_divisors 28)"  28   "sum-divisors 28"
zassert_eq "$(sum_proper_divisors 12)"  16   "sum-divisors 12 (abundant)"
zassert_eq "$(sum_proper_divisors 7)"   1    "sum-divisors 7 (prime)"
zassert_eq "$(classify 6)"   "perfect"    "6 perfect"
zassert_eq "$(classify 28)"  "perfect"    "28 perfect"
zassert_eq "$(classify 12)"  "abundant"   "12 abundant"
zassert_eq "$(classify 7)"   "deficient"  "7 deficient (prime)"
zassert_eq "$ab"             22           "abundant count 1..100"
ztest_run
