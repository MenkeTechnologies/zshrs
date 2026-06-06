#!/usr/bin/env zshrs
# Segment tree — O(log n) range-sum + point update.

typeset -a SEG
typeset -i N=0

# 1-indexed; tree size = 4*n for safety.
build() {
    local -a arr
    arr=("$@")
    N=${#arr}
    SEG=()
    local sz=$(( 4 * N ))
    local i
    for ((i=1; i<=sz; i++)); do SEG[i]=0; done
    _build 1 1 $N arr
}

_build() {
    local node=$1 l=$2 r=$3 arr_name=$4
    if (( l == r )); then
        local val
        eval "val=\${${arr_name}[$l]}"
        SEG[node]=$val
        return
    fi
    local mid=$(( (l + r) / 2 ))
    _build $((2*node)) $l $mid $arr_name
    _build $((2*node + 1)) $((mid + 1)) $r $arr_name
    SEG[node]=$(( SEG[2*node] + SEG[2*node + 1] ))
}

# range sum [ql..qr], inclusive.
query() {
    _query 1 1 $N $1 $2
}

_query() {
    local node=$1 l=$2 r=$3 ql=$4 qr=$5
    if (( qr < l || ql > r )); then echo 0; return; fi
    if (( ql <= l && r <= qr )); then
        echo ${SEG[node]}
        return
    fi
    local mid=$(( (l + r) / 2 ))
    local lsum=$(_query $((2*node)) $l $mid $ql $qr)
    local rsum=$(_query $((2*node + 1)) $((mid + 1)) $r $ql $qr)
    echo $(( lsum + rsum ))
}

# Point update: arr[i] = val.
update() {
    _update 1 1 $N $1 $2
}

_update() {
    local node=$1 l=$2 r=$3 i=$4 val=$5
    if (( l == r )); then
        SEG[node]=$val
        return
    fi
    local mid=$(( (l + r) / 2 ))
    if (( i <= mid )); then
        _update $((2*node)) $l $mid $i $val
    else
        _update $((2*node + 1)) $((mid + 1)) $r $i $val
    fi
    SEG[node]=$(( SEG[2*node] + SEG[2*node + 1] ))
}

input=(3 1 4 1 5 9 2 6 5 3 5 8 9 7 9 3)
echo "── input array (n=${#input}) ──"
echo "  ${input[*]}"

build "${input[@]}"
echo "  segtree built (size ${#SEG})"

echo
echo "── range sum queries ──"
queries=("1 5" "3 8" "1 16" "7 10" "5 5" "1 16")
for q in "${queries[@]}"; do
    set -- ${=q}
    ql=$1; qr=$2
    s=$(query $ql $qr)
    # Brute-force verify.
    bf=0
    for ((i=ql; i<=qr; i++)); do
        (( bf += input[i] ))
    done
    mark="✓"
    [[ $s != $bf ]] && mark="✗"
    printf "  sum[%2d..%2d] = %3d  (brute=%3d) %s\n" $ql $qr $s $bf $mark
done

echo
echo "── point updates ──"
update 3 100
input[3]=100
echo "  set arr[3] = 100"
s=$(query 1 5)
bf=0
for ((i=1; i<=5; i++)); do (( bf += input[i] )); done
printf "  sum[1..5] = %d (brute=%d)\n" $s $bf

update 16 0
input[16]=0
echo "  set arr[16] = 0"
s=$(query 1 16)
bf=0
for ((i=1; i<=16; i++)); do (( bf += input[i] )); done
printf "  sum[1..16] = %d (brute=%d)\n" $s $bf

echo
echo "── stress test (1000 random queries) ──"
RANDOM=42
mismatches=0
for ((iter=0; iter<200; iter++)); do
    ql=$(( RANDOM % N + 1 ))
    qr=$(( ql + RANDOM % (N - ql + 1) ))
    s=$(query $ql $qr)
    bf=0
    for ((i=ql; i<=qr; i++)); do (( bf += input[i] )); done
    (( s != bf )) && (( mismatches++ ))
done
echo "  200 random queries, mismatches: $mismatches"

echo
echo "── tree memory ──"
echo "  N = $N"
echo "  SEG entries = ${#SEG} (≈ 4N = $((4*N)))"

# === ztest assertions ===
# Final state after the two point updates above
zassert_eq "$N"        16   "tree size"
zassert_eq "${#SEG}"   64   "SEG ≈ 4N entries"
zassert_eq "$(query 1 16)" 173 "sum after final updates"
zassert_eq "$mismatches" 0  "200 random queries match brute force"
# Rebuild fresh + run primitive queries
build 1 2 3 4 5
zassert_eq "$N"              5   "N after rebuild"
zassert_eq "$(query 1 5)"    15  "sum 1..5 = 15"
zassert_eq "$(query 1 1)"    1   "single-element query"
zassert_eq "$(query 2 4)"    9   "range sum 2..4 = 2+3+4"
update 3 100
zassert_eq "$(query 1 5)"    112 "sum after update: 1+2+100+4+5"
zassert_eq "$(query 3 3)"    100 "point query reflects update"
ztest_run
