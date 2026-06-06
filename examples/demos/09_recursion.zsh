#!/usr/bin/env zshrs
# Recursion — factorial, gcd, power.

factorial() {
    if (( $1 <= 1 )); then
        echo 1
    else
        local prev=$(factorial $(( $1 - 1 )))
        echo $(( $1 * prev ))
    fi
}

gcd() {
    if (( $2 == 0 )); then
        echo $1
    else
        gcd $2 $(( $1 % $2 ))
    fi
}

power() {
    if (( $2 == 0 )); then
        echo 1
    else
        local prev=$(power $1 $(( $2 - 1 )))
        echo $(( $1 * prev ))
    fi
}

echo "── factorial ──"
for n in 0 1 2 3 5 7 10; do
    echo "$n! = $(factorial $n)"
done

echo "── gcd ──"
echo "gcd(48, 36) = $(gcd 48 36)"
echo "gcd(100, 75) = $(gcd 100 75)"
echo "gcd(7, 13) = $(gcd 7 13)"

echo "── power ──"
echo "2^0 = $(power 2 0)"
echo "2^4 = $(power 2 4)"
echo "3^5 = $(power 3 5)"

# === ztest assertions ===
zassert_eq "$(factorial 0)"  1       "0!"
zassert_eq "$(factorial 1)"  1       "1!"
zassert_eq "$(factorial 5)"  120     "5!"
zassert_eq "$(factorial 7)"  5040    "7!"
zassert_eq "$(factorial 10)" 3628800 "10!"
zassert_eq "$(gcd 48 36)"    12      "gcd(48,36)"
zassert_eq "$(gcd 100 75)"   25      "gcd(100,75)"
zassert_eq "$(gcd 7 13)"     1       "gcd coprime"
zassert_eq "$(power 2 0)"    1       "2^0"
zassert_eq "$(power 2 4)"    16      "2^4"
zassert_eq "$(power 3 5)"    243     "3^5"
ztest_run
