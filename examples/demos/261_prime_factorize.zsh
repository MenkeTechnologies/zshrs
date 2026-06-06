#!/usr/bin/env zshrs
# Prime factorization — trial division + canonical form.

factorize() {
    local n=$1 p=2 r="" cnt
    if (( n < 2 )); then echo "$n=()"; return; fi
    while (( p * p <= n )); do
        cnt=0
        while (( n % p == 0 )); do
            (( cnt++ ))
            (( n /= p ))
        done
        if (( cnt > 0 )); then
            if (( cnt == 1 )); then
                r+="$p × "
            else
                r+="$p^$cnt × "
            fi
        fi
        (( p++ ))
    done
    if (( n > 1 )); then r+="$n"; else r=${r% × }; fi
    echo "$r"
}

prime_count() {
    local n=$1 c=0 p=2 cnt
    while (( p * p <= n )); do
        cnt=0
        while (( n % p == 0 )); do (( cnt++ )); (( n /= p )); done
        (( c += cnt ))
        (( p++ ))
    done
    if (( n > 1 )); then (( c++ )); fi
    echo $c
}

distinct_count() {
    local n=$1 d=0 p=2
    while (( p * p <= n )); do
        if (( n % p == 0 )); then
            (( d++ ))
            while (( n % p == 0 )); do (( n /= p )); done
        fi
        (( p++ ))
    done
    if (( n > 1 )); then (( d++ )); fi
    echo $d
}

echo "── factorize 1..30 ──"
for n in {1..30}; do
    printf "%3d = %s\n" $n "$(factorize $n)"
done

echo
echo "── larger numbers ──"
for n in 360 1024 65537 720720 1000000 999983; do
    printf "%7d = %s   (Ω=%d, ω=%d)\n" $n "$(factorize $n)" "$(prime_count $n)" "$(distinct_count $n)"
done

echo
echo "── Mersenne-ish (2^k - 1) ──"
for k in 2 3 4 5 6 7 8 11 13; do
    n=$(( 2**k - 1 ))
    printf "  2^%d-1 = %5d = %s\n" $k $n "$(factorize $n)"
done

echo
echo "── factorial smooth-ness ──"
fact=1
for n in {1..10}; do
    (( fact *= n ))
    printf "  %2d! = %7d = %s\n" $n $fact "$(factorize $fact)"
done

# === ztest assertions ===
zassert_eq "$(factorize 2)"   "2"             "2 is prime"
zassert_eq "$(factorize 6)"   "2 × 3"         "6 = 2 × 3"
zassert_eq "$(factorize 12)"  "2^2 × 3"       "12 = 2² × 3"
zassert_eq "$(factorize 30)"  "2 × 3 × 5"     "30 = 2 × 3 × 5"
zassert_eq "$(factorize 360)" "2^3 × 3^2 × 5" "360 = 2³ × 3² × 5"
zassert_eq "$(factorize 1024)" "2^10"         "1024 = 2¹⁰"
zassert_eq "$(factorize 65537)" "65537"       "65537 is prime"
zassert_eq "$(prime_count 12)"  "3"           "Ω(12) = 3"
zassert_eq "$(prime_count 1024)" "10"         "Ω(1024) = 10"
zassert_eq "$(distinct_count 12)" "2"         "ω(12) = 2"
zassert_eq "$(distinct_count 30)" "3"         "ω(30) = 3"
zassert_eq "$(distinct_count 1024)" "1"       "ω(1024) = 1"
zassert_eq "$(factorize 255)" "3 × 5 × 17"    "2⁸-1 = 3 × 5 × 17"
zassert_eq "$(factorize 8191)" "8191"         "2¹³-1 = 8191 (Mersenne prime)"
ztest_run
