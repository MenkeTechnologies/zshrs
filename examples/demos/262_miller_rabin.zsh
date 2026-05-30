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
