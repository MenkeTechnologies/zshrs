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

# === ztest assertions ===
# Basic counter
seen=()
for ((i = 0; i < 5; i++)); do seen+=("$i"); done
zassert_eq "${(j: :)seen}" "0 1 2 3 4" "C-style basic counter"
# Descending
seen=()
for ((i = 10; i > 0; i -= 2)); do seen+=("$i"); done
zassert_eq "${(j: :)seen}" "10 8 6 4 2" "C-style descending step"
# Multi-counter
sums=()
for ((i = 0, j = 10; i < 5; i++, j--)); do sums+=("$((i+j))"); done
zassert_eq "${(j: :)sums}" "10 10 10 10 10" "multi-counter comma"
# Triangular
sum=0
for ((n = 1; n <= 10; n++)); do (( sum += n )); done
zassert_eq "$sum" 55 "T(10) = 55"
# Compound cond
seen=()
for ((i = 0; i < 10 && i * i < 50; i++)); do seen+=("$i"); done
zassert_eq "${(j: :)seen}" "0 1 2 3 4 5 6 7" "compound cond i*i<50"
# Primes
primes=()
isprime2() {
    local n=$1 k
    (( n < 2 )) && return 1
    for ((k = 2; k * k <= n; k++)); do (( n % k == 0 )) && return 1; done
    return 0
}
for ((n = 2, c = 0; c < 5; n++)); do
    if isprime2 $n; then primes+=("$n"); (( c++ )); fi
done
zassert_eq "${(j: :)primes}" "2 3 5 7 11" "first 5 primes"
ztest_run

