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
