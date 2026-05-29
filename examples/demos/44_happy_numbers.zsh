#!/usr/bin/env zshrs
# Happy numbers — sum-of-squares-of-digits iteration ends at 1.

sum_sq_digits() {
    local n=$1 s=0 d
    while (( n > 0 )); do
        d=$(( n % 10 ))
        (( s += d * d ))
        (( n /= 10 ))
    done
    echo $s
}

is_happy() {
    local n=$1
    typeset -A seen
    while (( n != 1 )); do
        [[ -n ${seen[$n]+x} ]] && return 1
        seen[$n]=1
        n=$(sum_sq_digits $n)
    done
    return 0
}

echo "── happy numbers 1..50 ──"
happy=()
for ((n = 1; n <= 50; n++)); do
    if is_happy $n; then happy+=($n); fi
done
echo "${happy[@]}"
echo "count: ${#happy[@]}"

# Build the trajectory in a string buffer (avoids `echo -n` + $(…)
# interleaving issues; see also 31_stack.zsh for the subshell-mutation
# pattern).
echo "── 19 trajectory (happy) ──"
buf="19"
n=19
while (( n != 1 )); do
    n=$(sum_sq_digits $n)
    buf+=" → $n"
done
echo "$buf"

echo "── 4 trajectory (unhappy, cycles) ──"
buf="4"
n=4
typeset -A seen
seen[$n]=1
while true; do
    n=$(sum_sq_digits $n)
    buf+=" → $n"
    if [[ -n ${seen[$n]+x} ]]; then
        echo "$buf (cycle)"
        break
    fi
    seen[$n]=1
done
