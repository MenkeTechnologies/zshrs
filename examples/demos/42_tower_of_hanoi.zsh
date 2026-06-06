#!/usr/bin/env zshrs
# Tower of Hanoi — classic recursive solution; prints move sequence.

hanoi() {
    local n=$1 from=$2 to=$3 via=$4
    if (( n == 1 )); then
        echo "move disk 1 from $from to $to"
        return
    fi
    hanoi $(( n - 1 )) $from $via $to
    echo "move disk $n from $from to $to"
    hanoi $(( n - 1 )) $via $to $from
}

echo "── n=3 disks A→C via B ──"
hanoi 3 A C B
echo "── total moves (n=3): $(( 2**3 - 1 )) ──"

echo "── n=4 disks A→C via B ──"
hanoi 4 A C B
echo "── total moves (n=4): $(( 2**4 - 1 )) ──"

# === ztest assertions ===
# Count moves for n=3.
moves3=$(hanoi 3 A C B | wc -l | tr -d ' ')
zassert_eq "$moves3" "7"   "n=3 needs 2^3-1=7 moves"
moves4=$(hanoi 4 A C B | wc -l | tr -d ' ')
zassert_eq "$moves4" "15"  "n=4 needs 2^4-1=15 moves"
moves5=$(hanoi 5 A C B | wc -l | tr -d ' ')
zassert_eq "$moves5" "31"  "n=5 needs 2^5-1=31 moves"
moves1=$(hanoi 1 A C B | wc -l | tr -d ' ')
zassert_eq "$moves1" "1"   "n=1 needs 1 move"
# First move for n=3 with A->C via B is disk 1 from A to C
first3=$(hanoi 3 A C B | head -1)
zassert_eq "$first3" "move disk 1 from A to C" "first move with odd n: A→C"
# Last move is always the biggest disk going to final
last3=$(hanoi 3 A C B | tail -1)
zassert_eq "$last3"  "move disk 1 from A to C" "last move n=3"
ztest_run
