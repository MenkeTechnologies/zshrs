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
