#!/usr/bin/env zshrs
# Miller-Rabin probabilistic primality test (deterministic for small n).

# Modular exponentiation: (base^exp) mod m.
modpow() {
    local base=$1 exp=$2 mod=$3
    local r=1
    (( base %= mod ))
    while (( exp > 0 )); do
        if (( exp & 1 )); then
            (( r = r * base % mod ))
        fi
        (( exp >>= 1 ))
        (( base = base * base % mod ))
    done
    echo $r
}

# Miller-Rabin witness check.
mr_witness() {
    local a=$1 n=$2 d=$3 r=$4
    local x t
    x=$(modpow $a $d $n)
    if (( x == 1 || x == n - 1 )); then return 0; fi
    for ((t=1; t<r; t++)); do
        x=$(modpow $x 2 $n)
        if (( x == n - 1 )); then return 0; fi
    done
    return 1
}

is_prime() {
    local n=$1
    if (( n < 2 )); then return 1; fi
    if (( n == 2 || n == 3 )); then return 0; fi
    if (( n % 2 == 0 )); then return 1; fi
    # Write n-1 = d * 2^r with d odd.
    local d=$(( n - 1 )) r=0
    while (( d % 2 == 0 )); do (( d >>= 1 )); (( r++ )); done
    # Deterministic witnesses for n < 3,215,031,751.
    local witnesses=(2 3 5 7)
    local a
    for a in "${witnesses[@]}"; do
        if (( a >= n )); then continue; fi
        if ! mr_witness $a $n $d $r; then
            return 1
        fi
    done
    return 0
}

echo "── small primes (2..50) ──"
out=""
for n in {2..50}; do
    if is_prime $n; then out+="$n "; fi
done
echo "  $out"

echo
echo "── classic primes ──"
for n in 97 101 7919 65521 65537 999983; do
    if is_prime $n; then
        printf "  %10d : prime\n" $n
    else
        printf "  %10d : composite\n" $n
    fi
done

echo
echo "── classic composites ──"
for n in 91 1729 561 1105 8911 561; do
    if is_prime $n; then
        printf "  %5d : prime (FALSE POS!)\n" $n
    else
        printf "  %5d : composite\n" $n
    fi
done
echo "(1729 = 7×13×19; 561 = 3×11×17 — Carmichael numbers)"

echo
echo "── count primes in [1, N] ──"
for upper in 10 50 100 200 500; do
    c=0
    for ((n=2; n<=upper; n++)); do
        if is_prime $n; then (( c++ )); fi
    done
    printf "  π(%4d) = %3d\n" $upper $c
done

echo
echo "── twin primes (p, p+2 both prime) under 100 ──"
out=""
for ((n=3; n<100; n++)); do
    if is_prime $n; then
        if is_prime $((n + 2)); then
            out+="($n,$((n+2))) "
        fi
    fi
done
echo "  $out"

# === ztest assertions ===
zassert_eq "$(modpow 2 10 1000)" "24"   "2^10 mod 1000"
zassert_eq "$(modpow 3 5 7)"     "5"    "3^5 mod 7 = 243 mod 7 = 5"
zassert_eq "$(modpow 7 0 13)"    "1"    "anything^0 = 1"
if is_prime 2;       then zassert_ok 1 "2 is prime"; else zassert_ok 0 "2 is prime"; fi
if is_prime 97;      then zassert_ok 1 "97 is prime"; else zassert_ok 0 "97 is prime"; fi
if is_prime 7919;    then zassert_ok 1 "7919 is prime"; else zassert_ok 0 "7919 is prime"; fi
if is_prime 65537;   then zassert_ok 1 "65537 is Fermat prime"; else zassert_ok 0 "65537 is Fermat prime"; fi
if is_prime 999983;  then zassert_ok 1 "999983 is prime"; else zassert_ok 0 "999983 is prime"; fi
if is_prime 1;       then zassert_ok 0 "1 is NOT prime"; else zassert_ok 1 "1 is NOT prime"; fi
if is_prime 91;      then zassert_ok 0 "91 = 7×13 composite"; else zassert_ok 1 "91 composite"; fi
if is_prime 561;     then zassert_ok 0 "561 Carmichael not flagged prime"; else zassert_ok 1 "561 Carmichael correctly composite"; fi
if is_prime 1729;    then zassert_ok 0 "1729 Carmichael correctly composite"; else zassert_ok 1 "1729 composite"; fi
# π(100) = 25 (known)
pi_100=0
for ((n=2; n<=100; n++)); do is_prime $n && (( pi_100++ )); done
zassert_eq "$pi_100" "25" "π(100) = 25"
ztest_run
