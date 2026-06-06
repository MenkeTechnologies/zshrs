#!/usr/bin/env zshrs
# Functions — local vars, return codes, multiple args.
greet() {
    local who=$1
    echo "Hello, $who!"
}

sum() {
    local total=0
    local n
    for n in "$@"; do
        (( total += n ))
    done
    echo $total
}

max() {
    local best=$1
    shift
    local x
    for x in "$@"; do
        (( x > best )) && best=$x
    done
    echo $best
}

is_positive() {
    (( $1 > 0 ))
}

greet world
greet zshrs

echo "sum 1..5 = $(sum 1 2 3 4 5)"
echo "sum 10 20 30 = $(sum 10 20 30)"

echo "max(3 1 4 1 5 9 2 6) = $(max 3 1 4 1 5 9 2 6)"

if is_positive 7; then echo "7 is positive"; fi
if is_positive -3; then echo "should not print"; else echo "-3 is not positive"; fi

# Local vs outer scope
outer=outer_val
demo_scope() {
    local outer=inner_val
    echo "inside: $outer"
}
demo_scope
echo "outside: $outer"

# === ztest assertions ===
zassert_eq "$(greet world)"           "Hello, world!" "greet world"
zassert_eq "$(greet zshrs)"           "Hello, zshrs!" "greet zshrs"
zassert_eq "$(sum 1 2 3 4 5)"         15              "sum 1..5"
zassert_eq "$(sum 10 20 30)"          60              "sum 10 20 30"
zassert_eq "$(max 3 1 4 1 5 9 2 6)"   9               "max varargs"
if is_positive 7; then zassert_ok 1 "is_positive 7"; else zassert_ok 0 "is_positive 7"; fi
if is_positive -3; then zassert_ok 0 "is_positive -3 negated"; else zassert_ok 1 "is_positive -3 negated"; fi
zassert_eq "$outer"                   "outer_val"     "local scope did not leak"
ztest_run
