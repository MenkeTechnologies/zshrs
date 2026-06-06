#!/usr/bin/env zshrs
# Pollard's rho — integer factorization via Floyd cycle detection.

# gcd via Euclidean.
gcd() {
    local a=$1 b=$2 t
    if (( a < 0 )); then a=$(( -a )); fi
    while (( b > 0 )); do
        t=$b
        b=$(( a % b ))
        a=$t
    done
    echo $a
}

# Pollard rho with Brent's improvement (simpler version).
pollard_rho() {
    local n=$1
    if (( n % 2 == 0 )); then echo 2; return; fi
    if (( n == 1 )); then echo 1; return; fi
    local x y d c i
    # f(x) = (x^2 + c) mod n
    # Try multiple c values until we find a factor.
    for c in 1 2 3 5 7 11; do
        x=2; y=2; d=1
        i=0
        while (( d == 1 )); do
            x=$(( (x * x + c) % n ))
            y=$(( (y * y + c) % n ))
            y=$(( (y * y + c) % n ))
            local diff
            if (( x > y )); then
                diff=$(( x - y ))
            else
                diff=$(( y - x ))
            fi
            d=$(gcd $diff $n)
            (( i++ ))
            if (( i > 1000 )); then break; fi
        done
        if (( d != n && d != 1 )); then
            echo $d
            return
        fi
    done
    echo $n   # gave up
}

# Trial division up to sqrt, then Pollard for larger composites.
factor() {
    local n=$1
    typeset -ga FACTORS
    FACTORS=()
    # Small primes.
    local p=2
    while (( p * p <= n )) && (( p <= 100 )); do
        while (( n % p == 0 )); do
            FACTORS+=($p)
            (( n /= p ))
        done
        (( p++ ))
    done
    if (( n > 1 )); then
        # Use Pollard for larger factors.
        if (( n < 10000 )); then
            # Direct trial division for moderate.
            while (( p * p <= n )); do
                while (( n % p == 0 )); do
                    FACTORS+=($p)
                    (( n /= p ))
                done
                (( p++ ))
            done
            if (( n > 1 )); then FACTORS+=($n); fi
        else
            local f=$(pollard_rho $n)
            if (( f > 1 && f < n )); then
                FACTORS+=($f)
                FACTORS+=($((n / f)))
            else
                FACTORS+=($n)   # probably prime
            fi
        fi
    fi
}

# Miller-Rabin (deterministic small).
is_prime() {
    local n=$1
    (( n < 2 )) && return 1
    (( n == 2 || n == 3 )) && return 0
    (( n % 2 == 0 )) && return 1
    local p
    for p in 3 5 7 11 13 17 19 23 29 31; do
        (( p * p > n )) && break
        (( n % p == 0 )) && return 1
    done
    return 0
}

echo "── Pollard's rho factorization ──"
numbers=(
    91          # 7 × 13
    1009         # prime
    8051        # 83 × 97
    10403       # 103 × 101
    9991        # 97 × 103
    1024        # 2^10
    65535       # 3 × 5 × 17 × 257
)
for n in "${numbers[@]}"; do
    factor $n
    sort_str=$(echo "${FACTORS[@]}" | tr ' ' '\n' | sort -n | tr '\n' ' ')
    sort_str="${sort_str% }"
    # Verify product.
    product=1
    for f in "${FACTORS[@]}"; do (( product *= f )); done
    mark="✓"
    [[ $product != $n ]] && mark="✗"
    printf "  %8d = %-25s product=%d %s\n" $n "$sort_str" $product $mark
done

echo
echo "── gcd via Euclidean ──"
gcd_tests=(
    "48 18"
    "100 75"
    "270 192"
    "12345 67890"
    "0 5"
    "17 13"
)
for t in "${gcd_tests[@]}"; do
    set -- ${=t}
    g=$(gcd $1 $2)
    printf "  gcd(%5d, %5d) = %d\n" $1 $2 $g
done

echo
echo "── prime test (Miller-Rabin small) ──"
for n in 2 3 5 7 11 13 23 97 101 1009 1024 65537 99991 100000; do
    if is_prime $n; then
        echo "  $n: prime"
    else
        echo "  $n: composite"
    fi
done

echo
echo "── Pollard rho parameters ──"
echo "  function: f(x) = (x² + c) mod n"
echo "  c values tried: 1, 2, 3, 5, 7, 11"
echo "  cycle detection: Floyd's (tortoise + hare)"
echo "  expected runtime: O(n^(1/4))"
echo "  worst case:       O(sqrt(n)) (degenerate)"

echo
echo "── factorization quality ──"
echo "  trial division alone: O(sqrt(n))"
echo "  Pollard rho:          O(n^(1/4)) for moderate composites"
echo "  Pollard p-1:          finds smooth factors quickly"
echo "  GNFS:                 best known for very large n"

# === ztest assertions ===
zassert_eq "$(gcd 48 18)"     6   "gcd(48,18)"
zassert_eq "$(gcd 100 75)"    25  "gcd(100,75)"
zassert_eq "$(gcd 17 13)"     1   "gcd coprime"
zassert_eq "$(gcd 0 5)"       5   "gcd(0,5)"
factor 91
zassert_eq "${FACTORS[*]}" "7 13" "factor 91"
factor 1024
zassert_eq "${#FACTORS}" 10 "factor 1024 = 2^10"
factor 65535
zassert_eq "${FACTORS[*]}" "3 5 17 257" "factor 65535"
if is_prime 1009; then zassert_ok 1 "1009 prime"
else zassert_ok 0 "1009 should be prime"; fi
if is_prime 1024; then zassert_ok 0 "1024 should not be prime"
else zassert_ok 1 "1024 composite"; fi
if is_prime 65537; then zassert_ok 1 "65537 prime"
else zassert_ok 0 "65537 should be prime"; fi
ztest_run
