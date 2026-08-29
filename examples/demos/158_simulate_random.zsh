#!/usr/bin/env zshrs
# Deterministic-seed PRNG via $RANDOM.
# Ported from Src/params.c randomgetfn / randomsetfn.

# Set a seed (RANDOM= seeds via srand()).
RANDOM=42

echo "── deterministic seed=42 sequence ──"
for i in 1 2 3 4 5 6 7 8 9 10; do
    printf "%d " $RANDOM
done
echo

echo "── reset seed and re-test ──"
RANDOM=42
for i in 1 2 3; do
    printf "%d " $RANDOM
done
echo "(same as first 3 above)"

echo "── range 1..6 (dice) ──"
RANDOM=100
for i in {1..10}; do
    printf "%d " $(( RANDOM % 6 + 1 ))
done
echo

echo "── shuffle an array via Fisher-Yates ──"
RANDOM=7
shuffle() {
    local -a arr=("$@")
    local n=${#arr[@]} i j tmp
    for ((i = n; i > 1; i--)); do
        j=$(( RANDOM % i + 1 ))
        tmp=${arr[i]}
        arr[i]=${arr[j]}
        arr[j]=$tmp
    done
    echo "${arr[@]}"
}
shuffle alpha beta gamma delta epsilon
shuffle 1 2 3 4 5 6 7 8 9 10

echo "── random sampling: pick 3 of 7 ──"
sample() {
    local n=$1; shift
    local -a pool=("$@")
    local -a out
    for ((i = 0; i < n; i++)); do
        if (( ${#pool[@]} == 0 )); then break; fi
        local idx=$(( RANDOM % ${#pool[@]} + 1 ))
        out+=("${pool[idx]}")
        # Remove picked.
        pool=("${pool[@]:0:$((idx-1))}" "${pool[@]:$idx}")
    done
    echo "${out[@]}"
}
RANDOM=13
sample 3 apple banana cherry date elderberry fig grape

echo "── monte-carlo π estimate (deterministic from seed) ──"
RANDOM=1
inside=0; total=10000
# Smaller N for CI; scale: 0..32767, square is 32767^2.
for ((i = 0; i < total; i++)); do
    local x=$(( RANDOM ))
    local y=$(( RANDOM ))
    # (x/M)^2 + (y/M)^2 <= 1 → x*x + y*y <= M*M
    local lhs=$(( x*x + y*y ))
    local rhs=$(( 32767 * 32767 ))
    (( lhs <= rhs )) && (( inside++ ))
done
echo "π estimate: $(( inside * 4 * 1000 / total )) / 1000"

# === ztest assertions ===
RANDOM=42; a1=$RANDOM; a2=$RANDOM
RANDOM=42; b1=$RANDOM; b2=$RANDOM
zassert_eq "$a1" "$b1"  "RANDOM=42 reproduces its first value"
zassert_eq "$a2" "$b2"  "RANDOM=42 reproduces its second value"
RANDOM=100; die=$(( RANDOM % 6 + 1 ))
zassert_ge "$die" 1     "dice roll >= 1"
zassert_le "$die" 6     "dice roll <= 6"
RANDOM=7
shuffled=( ${=$(shuffle a b c d e)} )
zassert_eq "${#shuffled}" 5                        "shuffle keeps every element"
zassert_eq "${(j: :)${(o)shuffled[@]}}" "a b c d e" "shuffle is a permutation"
RANDOM=13
picked=( ${=$(sample 3 apple banana cherry date elderberry fig grape)} )
zassert_eq "${#picked}" 3                          "sample 3 returns three items"
zassert_eq "${#${(u)picked[@]}}" 3                  "sample never repeats an item"
ztest_run
