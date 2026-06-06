#!/usr/bin/env zshrs
# Fibonacci — iterative O(n), pair-update O(n) producing the whole sequence.

# Single value via two-counter update.
fib_iter() {
    local n=$1 a=0 b=1 i tmp
    for ((i = 0; i < n; i++)); do
        tmp=$(( a + b ))
        a=$b
        b=$tmp
    done
    echo $a
}

# Whole sequence printed in one pass (no subshell recursion).
fib_seq() {
    local n=$1 a=0 b=1 i tmp
    for ((i = 0; i <= n; i++)); do
        echo $a
        tmp=$(( a + b ))
        a=$b
        b=$tmp
    done
}

echo "── single-value iterative ──"
for i in 0 1 2 3 4 5 10 15 20; do
    printf "fib(%2d) = %d\n" $i $(fib_iter $i)
done

echo "── full sequence 0..20 ──"
fib_seq 20

echo "── sum of fib(0..15) ──"
total=0
for v in $(fib_seq 15); do
    (( total += v ))
done
echo "sum = $total"

# === ztest assertions ===
zassert_eq "$(fib_iter 0)"  0     "fib(0)"
zassert_eq "$(fib_iter 1)"  1     "fib(1)"
zassert_eq "$(fib_iter 5)"  5     "fib(5)"
zassert_eq "$(fib_iter 10)" 55    "fib(10)"
zassert_eq "$(fib_iter 15)" 610   "fib(15)"
zassert_eq "$(fib_iter 20)" 6765  "fib(20)"
# fib_seq 5 prints 0..5: 0 1 1 2 3 5
zassert_eq "$(fib_seq 5 | tr '\n' ' ')" "0 1 1 2 3 5 " "fib_seq 5"
# Sum F(0)..F(15) = F(17) - 1 = 1597 - 1 = 1596
zassert_eq "$total" 1596 "sum fib(0..15)"
ztest_run
