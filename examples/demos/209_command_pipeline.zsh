#!/usr/bin/env zshrs
# Composable command pipeline patterns.

map() {
    local fn=$1
    shift
    for x in "$@"; do
        $fn "$x"
    done
}

filter() {
    local pred=$1
    shift
    for x in "$@"; do
        if $pred "$x"; then echo "$x"; fi
    done
}

reduce() {
    local fn=$1 acc=$2
    shift 2
    for x in "$@"; do
        acc=$($fn "$acc" "$x")
    done
    echo "$acc"
}

# Sample fns.
double() { echo $(( $1 * 2 )); }
square() { echo $(( $1 * $1 )); }
add() { echo $(( $1 + $2 )); }
is_even() { (( $1 % 2 == 0 )); }
is_positive() { (( $1 > 0 )); }

echo "── map double over 1..5 ──"
mapped=( $(map double 1 2 3 4 5) )
echo "${mapped[@]}"

echo "── filter even ──"
nums=(1 2 3 4 5 6 7 8 9 10)
evens=( $(filter is_even "${nums[@]}") )
echo "${evens[@]}"

echo "── reduce sum ──"
sum=$(reduce add 0 1 2 3 4 5 6 7 8 9 10)
echo "sum 1..10 = $sum"

echo "── compose: square all evens then sum ──"
result=0
for n in "${nums[@]}"; do
    if is_even "$n"; then
        result=$(add $result $(square $n))
    fi
done
echo "sum of squares of evens = $result"

echo "── named pipeline ──"
pipeline() {
    local input=("${(@)@}")
    # Stage 1: filter positive.
    local stage1=()
    for x in "${input[@]}"; do
        is_positive "$x" && stage1+=("$x")
    done
    # Stage 2: double.
    local stage2=()
    for x in "${stage1[@]}"; do
        stage2+=("$(double "$x")")
    done
    # Stage 3: sum.
    reduce add 0 "${stage2[@]}"
}

input=(5 -3 7 -1 10 -2 8 -5)
result=$(pipeline "${input[@]}")
echo "pipeline on ${input[@]} → $result"

echo "── functional-style with shell pipes ──"
echo "(filter ≥3 then square)"
printf '%d\n' 1 2 3 4 5 6 7 8 | while read n; do
    if (( n >= 3 )); then
        echo $(( n * n ))
    fi
done | (sum=0; while read v; do (( sum += v )); done; echo "sum=$sum")

# === ztest assertions ===
zassert_eq "$(map double 1 2 3)" "$(printf '2\n4\n6')"   "map double 1 2 3"
zassert_eq "$(filter is_even 1 2 3 4 5 6)" "$(printf '2\n4\n6')" "filter even 1..6"
zassert_eq "$(reduce add 0 1 2 3 4 5)"     15           "reduce sum 1..5"
zassert_eq "$(reduce add 0 1 2 3 4 5 6 7 8 9 10)" 55    "reduce sum 1..10"
zassert_eq "$(double 7)"  14    "double 7"
zassert_eq "$(square 9)"  81    "square 9"
is_even 4 && r=1 || r=0; zassert_eq "$r" 1 "is_even 4"
is_even 5 && r=1 || r=0; zassert_eq "$r" 0 "is_even 5 → false"
zassert_eq "$result" 60        "named pipeline result on mixed sign input"
ztest_run
