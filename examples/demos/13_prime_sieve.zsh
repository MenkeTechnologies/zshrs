#!/usr/bin/env zshrs
# Sieve of Eratosthenes — primes up to N.
sieve() {
    local n=$1
    local is_prime=()
    local i j
    # Allocate n+1 slots, 1-based; slot k → number (k-1).
    for ((i = 0; i <= n; i++)); do
        is_prime+=(1)
    done
    # Mark 0 and 1 as not prime (slots 1 and 2).
    is_prime[1]=0
    is_prime[2]=0
    for ((i = 2; i * i <= n; i++)); do
        if (( is_prime[i + 1] )); then
            for ((j = i * i; j <= n; j += i)); do
                is_prime[j+1]=0
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

# === ztest assertions ===
zassert_eq "$(sieve 10)"   "2 3 5 7"                       "primes <= 10"
zassert_eq "$(sieve 20)"   "2 3 5 7 11 13 17 19"           "primes <= 20"
zassert_eq "$(sieve 30)"   "2 3 5 7 11 13 17 19 23 29"     "primes <= 30"
zassert_contains "$(sieve 100)" "97"                       "primes <= 100 ends at 97"
zassert_eq "$(sieve 2)"    "2"                             "smallest prime"
zassert_eq "$(sieve 1)"    ""                              "no primes <= 1"
# π(N) — count of primes ≤ N
zassert_eq "$(sieve 30  | wc -w | tr -d ' ')" "10" "π(30) = 10"
zassert_eq "$(sieve 100 | wc -w | tr -d ' ')" "25" "π(100) = 25"
zassert_eq "$(sieve 200 | wc -w | tr -d ' ')" "46" "π(200) = 46"
ztest_run
