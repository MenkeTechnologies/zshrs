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
