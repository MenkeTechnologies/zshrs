#!/usr/bin/env zshrs
# Traps and exit handlers — cleanup pattern.

tmpfile="/tmp/zshrs_demo_trap_$$"
cleanup() {
    rm -f "$tmpfile"
    echo "cleanup done"
}

trap cleanup EXIT

echo "── creating temp ──"
echo "scratch data" > "$tmpfile"
echo "tmpfile: $tmpfile"
echo "exists: $(test -f $tmpfile && echo yes || echo no)"

echo "── reading temp ──"
cat "$tmpfile"

echo "── normal exit fires trap ──"
