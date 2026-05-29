#!/usr/bin/env zshrs
# Sieve of Eratosthenes — primes up to N.
sieve() {
    local n=$1
    local is_prime=()
    local i j idx
    # Allocate n+1 slots, 1-based; slot k → number (k-1).
    for ((i = 0; i <= n; i++)); do
        is_prime+=(1)
    done
    # Mark 0 and 1 as not prime (slots 1 and 2).
    is_prime[1]=0
    is_prime[2]=0
    for ((i = 2; i * i <= n; i++)); do
        if (( is_prime[i + 1] )); then
            j=$(( i * i ))
            while (( j <= n )); do
                idx=$(( j + 1 ))
                is_prime[$idx]=0
                (( j += i ))
            done
        fi
    done
    local primes=()
    for ((i = 2; i <= n; i++)); do
        (( is_prime[i + 1] )) && primes+=($i)
    done
    echo "${primes[@]}"
}

echo "── primes ≤ 30 ──"
sieve 30

echo "── primes ≤ 100 ──"
sieve 100

echo "── prime count ≤ 200 ──"
count=$(sieve 200 | wc -w)
echo "π(200) = $count"
