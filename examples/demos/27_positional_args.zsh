#!/usr/bin/env zshrs
# Positional parameters — $1 $2 $@ $* $# and shift.
set -- alpha beta gamma delta epsilon

echo "── basic ──"
echo "count: $#"
echo "all: $*"
echo "first: $1"
echo "second: $2"
echo "fifth: $5"

echo "── iterate \"\$@\" ──"
for x in "$@"; do
    echo "arg: $x"
done

echo "── shift one ──"
shift
echo "after shift: count=$# first=$1"

echo "── shift two more ──"
shift 2
echo "after shift 2: count=$# all=$*"

echo "── reset and iterate by index ──"
set -- one two three
for ((i = 1; i <= $#; i++)); do
    echo "[$i] = ${(P)i}"
done

# === ztest assertions ===
# Snapshot final state after `set -- one two three`.
zassert_eq "$#"  3       "3 positional args after final set"
zassert_eq "$1"  "one"   "\$1"
zassert_eq "$2"  "two"   "\$2"
zassert_eq "$3"  "three" "\$3"
zassert_eq "$*"  "one two three"      "\$* joins with IFS"
# Exercise shift behavior in a subshell so we don't disturb outer state.
zassert_eq "$(set -- a b c d e; echo $#)"      5       "set -- builds 5-arg list"
zassert_eq "$(set -- a b c d e; shift; echo $#)"   4   "shift drops one"
zassert_eq "$(set -- a b c d e; shift 2; echo $1)" "c" "shift 2 lands on third"
star_in_sub=$(set -- a b c; echo "$*")
zassert_eq "$star_in_sub" "a b c"     "\$* joins in subshell"
ztest_run
