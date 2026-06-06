#!/usr/bin/env zshrs
# Fenwick (BIT) tree — prefix sums + point updates O(log n).

typeset -a BIT
N=0

bit_init() {
    local n=$1 i
    N=$n
    BIT=()
    for ((i=1; i<=n; i++)); do BIT[i]=0; done
}

# Point update: bit[i] += delta.
bit_update() {
    local i=$1 delta=$2
    while (( i <= N )); do
        (( BIT[i] += delta ))
        (( i += i & (-i) ))    # next: add lowest bit
    done
}

# Prefix sum [1..i].
bit_prefix() {
    local i=$1 s=0
    while (( i > 0 )); do
        (( s += BIT[i] ))
        (( i -= i & (-i) ))    # strip lowest bit
    done
    echo $s
}

# Range sum [l..r] = prefix(r) - prefix(l-1).
bit_range() {
    local l=$1 r=$2
    local pr=$(bit_prefix $r)
    local pl=$(bit_prefix $((l - 1)))
    echo $(( pr - pl ))
}

# Build from initial array.
bit_build() {
    local -a arr
    arr=("$@")
    bit_init ${#arr}
    local i
    for ((i=1; i<=N; i++)); do
        bit_update $i ${arr[i]}
    done
}

# Find smallest i where prefix(i) >= target (assumes non-neg values).
bit_lower_bound() {
    local target=$1
    local i=0 r=0
    local bit_n=$N pow=1
    # Find largest power of 2 ≤ N.
    while (( pow * 2 <= bit_n )); do (( pow *= 2 )); done
    while (( pow > 0 )); do
        if (( i + pow <= bit_n && BIT[i + pow] < target )); then
            (( i += pow ))
            (( target -= BIT[i] ))
        fi
        (( pow >>= 1 ))
    done
    echo $((i + 1))
}

input=(3 1 4 1 5 9 2 6 5 3 5 8 9 7 9 3)
echo "── input ──"
echo "  ${input[*]}"
echo "  n = ${#input}"

bit_build "${input[@]}"
echo "  BIT built (${#BIT} entries)"

echo
echo "── prefix sums ──"
for k in 1 4 8 12 16; do
    s=$(bit_prefix $k)
    bf=0
    for ((i=1; i<=k; i++)); do (( bf += input[i] )); done
    printf "  prefix(%2d) = %3d (brute=%3d) %s\n" $k $s $bf "$([[ $s == $bf ]] && echo ✓ || echo ✗)"
done

echo
echo "── range sums ──"
ranges=("2 7" "5 12" "1 16" "8 8" "10 14")
for r in "${ranges[@]}"; do
    set -- ${=r}
    s=$(bit_range $1 $2)
    bf=0
    for ((i=$1; i<=$2; i++)); do (( bf += input[i] )); done
    printf "  range[%2d..%2d] = %3d (brute=%3d) %s\n" $1 $2 $s $bf "$([[ $s == $bf ]] && echo ✓ || echo ✗)"
done

echo
echo "── point updates ──"
bit_update 5 -3
input[5]=$(( input[5] - 3 ))
echo "  arr[5] -= 3 (now ${input[5]})"
s=$(bit_range 1 10)
bf=0
for ((i=1; i<=10; i++)); do (( bf += input[i] )); done
printf "  range[1..10] = %d (brute=%d) %s\n" $s $bf "$([[ $s == $bf ]] && echo ✓ || echo ✗)"

bit_update 12 100
input[12]=$(( input[12] + 100 ))
echo "  arr[12] += 100 (now ${input[12]})"
s=$(bit_range 1 16)
bf=0
for ((i=1; i<=16; i++)); do (( bf += input[i] )); done
printf "  total = %d (brute=%d) %s\n" $s $bf "$([[ $s == $bf ]] && echo ✓ || echo ✗)"

echo
echo "── inversions count via BIT ──"
# Count pairs (i,j) i<j with a[i]>a[j].
arr=(8 4 2 1 5 7 3 6)
echo "  array: ${arr[*]}"
# Map to 1..n via sorted ranks.
sorted=("${(@n)arr}")
typeset -A rank
for ((i=1; i<=${#sorted}; i++)); do
    rank[${sorted[i]}]=$i
done
bit_init ${#arr}
inversions=0
for ((i=${#arr}; i>=1; i--)); do
    r=${rank[${arr[i]}]}
    # Count of already-seen elements with rank < r.
    s=$(bit_prefix $((r - 1)))
    (( inversions += s ))
    bit_update $r 1
done
echo "  inversions: $inversions"
# Brute-force verify.
bf=0
for ((i=1; i<=${#arr}; i++)); do
    for ((j=i+1; j<=${#arr}; j++)); do
        (( arr[i] > arr[j] )) && (( bf++ ))
    done
done
echo "  brute-force: $bf   $([[ $inversions == $bf ]] && echo ✓ || echo ✗)"

# === ztest assertions ===
zassert_eq "$inversions" 14    "inversions of (8 4 2 1 5 7 3 6) = 14"
zassert_eq "$inversions" "$bf" "BIT inversions = brute force"
# Fresh BIT for clean test
bit_build 1 2 3 4 5
zassert_eq "$N" 5                        "N=5 after fresh build"
zassert_eq "$(bit_prefix 5)" 15          "prefix(5) = 1+2+3+4+5"
zassert_eq "$(bit_prefix 3)" 6           "prefix(3) = 6"
zassert_eq "$(bit_range 2 4)" 9          "range(2,4) = 2+3+4"
zassert_eq "$(bit_range 1 1)" 1          "range(1,1)"
zassert_eq "$(bit_range 5 5)" 5          "range(5,5)"
bit_update 3 10
zassert_eq "$(bit_prefix 5)" 25          "after +10 on idx 3: prefix = 25"
zassert_eq "$(bit_range 3 3)" 13         "range(3,3) after +10"
# Lower-bound search
zassert_eq "$(bit_lower_bound 1)" 1      "lower_bound(1) = idx 1 (val 1)"
ztest_run
