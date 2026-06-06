#!/usr/bin/env zshrs
# FizzBuzz 1..30 — the classic.
for i in {1..30}; do
    if (( i % 15 == 0 )); then
        echo "FizzBuzz"
    elif (( i % 3 == 0 )); then
        echo "Fizz"
    elif (( i % 5 == 0 )); then
        echo "Buzz"
    else
        echo "$i"
    fi
done

# === ztest assertions ===
fizzbuzz() {
    if   (( $1 % 15 == 0 )); then echo FizzBuzz
    elif (( $1 % 3  == 0 )); then echo Fizz
    elif (( $1 % 5  == 0 )); then echo Buzz
    else                          echo $1
    fi
}
zassert_eq "$(fizzbuzz 1)"  1          "1"
zassert_eq "$(fizzbuzz 3)"  Fizz       "Fizz"
zassert_eq "$(fizzbuzz 5)"  Buzz       "Buzz"
zassert_eq "$(fizzbuzz 9)"  Fizz       "9 -> Fizz"
zassert_eq "$(fizzbuzz 10)" Buzz       "10 -> Buzz"
zassert_eq "$(fizzbuzz 15)" FizzBuzz   "15 -> FizzBuzz"
zassert_eq "$(fizzbuzz 30)" FizzBuzz   "30 -> FizzBuzz"
zassert_eq "$(fizzbuzz 22)" 22         "22 plain"
ztest_run
