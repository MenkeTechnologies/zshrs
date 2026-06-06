#!/usr/bin/env zshrs
# Armstrong (narcissistic) numbers — sum of d^k digits equals n.

power() {
    local b=$1 e=$2 r=1
    while (( e > 0 )); do
        (( r *= b ))
        (( e-- ))
    done
    echo $r
}

is_armstrong() {
    local n=$1
    local len=${#n}
    local digits=("${(s::)n}")
    local sum=0
    for d in "${digits[@]}"; do
        (( sum += $(power $d $len) ))
    done
    (( sum == n ))
}

echo "── armstrong numbers up to 1000 ──"
arms=()
for ((n = 1; n <= 1000; n++)); do
    if is_armstrong $n; then arms+=($n); fi
done
echo "${arms[@]}"
echo "count: ${#arms[@]}"

echo "── verify 153 = 1³+5³+3³ ──"
echo "1³=$(power 1 3), 5³=$(power 5 3), 3³=$(power 3 3)"
echo "sum = $(( 1+125+27 ))"

# === ztest assertions ===
zassert_eq "$(power 2 10)"   1024 "power 2^10"
zassert_eq "$(power 5 3)"    125  "power 5^3"
zassert_eq "$(power 1 0)"    1    "power x^0"
is_armstrong 153 && zassert_ok 1 "153 is armstrong" || zassert_ok 0 "153 is armstrong"
is_armstrong 10  && zassert_ok 0 "10 not armstrong" || zassert_ok 1 "10 not armstrong"
is_armstrong 407 && zassert_ok 1 "407 is armstrong" || zassert_ok 0 "407 is armstrong"
zassert_eq "${#arms[@]}"  13                                "armstrong count ≤1000"
zassert_eq "${arms[*]}"   "1 2 3 4 5 6 7 8 9 153 370 371 407" "armstrong list ≤1000"
zassert_eq $(( 1 + 125 + 27 )) 153 "1+125+27=153"
ztest_run
