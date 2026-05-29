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
