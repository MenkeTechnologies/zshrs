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

# === ztest assertions ===
zassert_ok    "$tmpfile"          "tmpfile var set"
zassert_match '^/tmp/'  "$tmpfile" "tmpfile is under /tmp"
zassert_ok    "$(test -f "$tmpfile" && echo y)" "tmpfile exists"
zassert_eq    "$(cat "$tmpfile")" "scratch data"  "tmpfile content"
zassert_contains "$tmpfile" "$$"  "tmpfile uses pid suffix"
ztest_run
