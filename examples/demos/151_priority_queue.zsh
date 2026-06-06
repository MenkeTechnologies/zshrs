#!/usr/bin/env zshrs
# Priority queue — sorted output via array sort flag.

typeset -a PQ

pq_push() { PQ+=("$1"); }
pq_size() { echo ${#PQ[@]}; }

echo "── push 7 values ──"
for v in 7 3 9 1 5 2 8; do
    pq_push $v
    echo "  pushed $v, size=$(pq_size)"
done

echo "── sorted output via (n) ──"
print -l ${(n)PQ}

echo "── reverse-sorted via (On) ──"
print -l ${(On)PQ}

echo "── smallest k via (n) + head ──"
print -l ${(n)PQ} | head -3

echo "── 'pop' one — get smallest, remove ──"
PQ=(100 50 25 75 10 90 33)
sorted=( ${(n)PQ} )
echo "smallest: ${sorted[1]}"
echo "remaining (sorted): ${sorted[2,-1]}"

# === ztest assertions ===
zassert_eq "${sorted[1]}"  "10"  "smallest after numeric sort"
zassert_eq "${sorted[-1]}" "100" "largest after numeric sort"
zassert_eq "${sorted[2,-1]}" "25 33 50 75 90 100" "remaining after pop"
PQ2=(7 3 9 1 5 2 8)
sorted2=( ${(n)PQ2} )
zassert_eq "${(j: :)sorted2}"  "1 2 3 5 7 8 9" "(n) ascending sort"
revs=( ${(On)PQ2} )
zassert_eq "${(j: :)revs}"     "9 8 7 5 3 2 1" "(On) descending sort"
zassert_eq "${#sorted2[@]}"    "7"             "sort preserves count"
# pq_size
PQ3=()
local -a PQ_save=( "${PQ[@]}" )
PQ=()
zassert_eq "$(pq_size)" "0" "pq_size empty"
pq_push 42
zassert_eq "$(pq_size)" "1" "pq_size 1 after push"
zassert_eq "${PQ[1]}" "42" "push deposits value"
PQ=( "${PQ_save[@]}" )
ztest_run
