#!/usr/bin/env zshrs
# Advanced C-style for loops — multi-counter, comma expressions, nesting.
# Ported from Src/parse.c parsefor + Src/exec.c execfor C-style branch.

echo "── basic ──"
for ((i = 0; i < 5; i++)); do
    echo "i=$i"
done

echo "── descending step ──"
for ((i = 10; i > 0; i -= 2)); do
    echo "i=$i"
done

echo "── multi-counter via comma ──"
for ((i = 0, j = 10; i < 5; i++, j--)); do
    printf "i=%d j=%d\n" $i $j
done

echo "── empty parts (init/cond/iter) ──"
i=0
for ((; i < 3;)); do
    echo "manual i=$i"
    (( i++ ))
done

echo "── nested c-style ──"
for ((i = 1; i <= 3; i++)); do
    for ((j = 1; j <= 3; j++)); do
        printf "(%d,%d) " $i $j
    done
    echo
done

echo "── compound condition ──"
for ((i = 0; i < 10 && i * i < 50; i++)); do
    printf "i=%d i²=%d\n" $i $((i*i))
done

echo "── triangular numbers ──"
sum=0
for ((n = 1; n <= 10; n++)); do
    (( sum += n ))
    printf "T(%d) = %d\n" $n $sum
done

echo "── interleaved primes-ish (just first N candidates) ──"
isprime() {
    local n=$1 i
    (( n < 2 )) && return 1
    for ((i = 2; i * i <= n; i++)); do
        (( n % i == 0 )) && return 1
    done
    return 0
}
for ((n = 2, count = 0; count < 10; n++)); do
    if isprime $n; then
        printf "%d " $n
        (( count++ ))
    fi
done
echo
