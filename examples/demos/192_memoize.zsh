#!/usr/bin/env zshrs
# Memoization wrapper — cache function results.

typeset -A MEMO_CACHE
typeset -i MEMO_HITS=0
typeset -i MEMO_MISSES=0

memoize() {
    local fn=$1
    local key="${fn}|$*"
    if [[ -n ${MEMO_CACHE[$key]+x} ]]; then
        (( MEMO_HITS++ ))
        echo "${MEMO_CACHE[$key]}"
        return
    fi
    (( MEMO_MISSES++ ))
    shift
    local result=$($fn "$@")
    MEMO_CACHE[$key]=$result
    echo "$result"
}

# Slow function — uncached fibonacci.
slow_fib() {
    local n=$1
    if (( n < 2 )); then echo $n; return; fi
    local a=$(slow_fib $((n-1)))
    local b=$(slow_fib $((n-2)))
    echo $((a + b))
}

# Memoized version.
memo_fib() {
    local n=$1
    local key="memo_fib|$n"
    if [[ -n ${MEMO_CACHE[$key]+x} ]]; then
        (( MEMO_HITS++ ))
        echo "${MEMO_CACHE[$key]}"
        return
    fi
    (( MEMO_MISSES++ ))
    local result
    if (( n < 2 )); then
        result=$n
    else
        local a=$(memo_fib $((n-1)))
        local b=$(memo_fib $((n-2)))
        result=$((a + b))
    fi
    MEMO_CACHE[$key]=$result
    echo "$result"
}

echo "── memoized fib(15) ──"
MEMO_CACHE=()
MEMO_HITS=0
MEMO_MISSES=0
result=$(memo_fib 10)
echo "fib(10) = $result"
echo "cache hits: $MEMO_HITS misses: $MEMO_MISSES"

echo "── fib(0..15) (uses cache from prior call) ──"
echo "calling 15 fibs:"
out=""
for i in {0..10}; do
    out+="$(memo_fib $i) "
done
echo "$out"
echo "after batch: hits=$MEMO_HITS misses=$MEMO_MISSES"
echo "cache size: ${#MEMO_CACHE[@]}"

echo "── memoize a different function (square) ──"
square() { echo $(( $1 * $1 )); }
MEMO_CACHE=()
MEMO_HITS=0
MEMO_MISSES=0
for n in 5 10 5 15 10 20 5 25; do
    echo "  square($n) = $(memoize square $n)"
done
echo "hits=$MEMO_HITS misses=$MEMO_MISSES"

echo "── cache eviction (manual) ──"
echo "size: ${#MEMO_CACHE[@]}"
MEMO_CACHE=()
echo "after clear: ${#MEMO_CACHE[@]}"
