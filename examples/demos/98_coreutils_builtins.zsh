#!/usr/bin/env zshrs
# zshrs's coreutils-as-builtins — cat/head/tail/sort/wc/cut/tr/seq run
# without fork. The host shell intercepts these names per the dispatch
# table in src/extensions/ext_builtins.rs (genuine extension, not a
# port of any Src/*.c).

tmpdir=/tmp/zshrs_coreutils_$$
mkdir -p "$tmpdir"
trap "rm -rf $tmpdir" EXIT

# Build a test file.
{
    echo "line 1"
    echo "line 2"
    echo "line 3"
    echo "line 4"
    echo "line 5"
} > "$tmpdir/data.txt"

echo "── cat ──"
cat "$tmpdir/data.txt"

echo "── head -n N ──"
head -n 2 "$tmpdir/data.txt"

echo "── tail -n N ──"
tail -n 2 "$tmpdir/data.txt"

echo "── wc -l (line count) ──"
wc -l < "$tmpdir/data.txt"

echo "── wc -w (word count) ──"
echo "the quick brown fox" | wc -w

echo "── sort ascending ──"
{ echo gamma; echo alpha; echo beta; } | sort

echo "── sort -r descending ──"
{ echo alpha; echo gamma; echo beta; } | sort -r

echo "── sort -n numeric ──"
{ echo 10; echo 2; echo 100; echo 1; } | sort -n

echo "── tr (translate) ──"
echo "Hello World" | tr 'a-z' 'A-Z'
echo "Hello World" | tr 'A-Z' 'a-z'

echo "── tr -d (delete chars) ──"
echo "h-e-l-l-o" | tr -d '-'

echo "── cut -d delim -f field ──"
echo "alice,30,admin" | cut -d, -f1
echo "alice,30,admin" | cut -d, -f2
echo "alice,30,admin" | cut -d, -f3

echo "── pipeline of builtins ──"
{ echo banana; echo apple; echo cherry; echo banana; } | sort -u | head -2

echo "── seq replacement (brace expansion) ──"
echo {1..5}

echo "── tee through file then back ──"
echo "broadcast" | tee "$tmpdir/copy.log" | head -1
echo "tee'd contents:"
cat "$tmpdir/copy.log"

# === ztest assertions ===
zassert_eq "$(head -n 2 "$tmpdir/data.txt")"   $'line 1\nline 2'  "head -n 2"
zassert_eq "$(tail -n 2 "$tmpdir/data.txt")"   $'line 4\nline 5'  "tail -n 2"
zassert_eq "$(wc -l < "$tmpdir/data.txt" | tr -d ' ')" "5"        "wc -l = 5"
zassert_eq "$(echo 'the quick brown fox' | wc -w | tr -d ' ')" "4" "wc -w = 4"
zassert_eq "$({ echo gamma; echo alpha; echo beta; } | sort)" $'alpha\nbeta\ngamma' "sort asc"
zassert_eq "$({ echo alpha; echo gamma; echo beta; } | sort -r)" $'gamma\nbeta\nalpha' "sort -r desc"
zassert_eq "$({ echo 10; echo 2; echo 100; echo 1; } | sort -n)" $'1\n2\n10\n100' "sort -n numeric"
zassert_eq "$(echo 'Hello World' | tr 'a-z' 'A-Z')" "HELLO WORLD" "tr upper"
zassert_eq "$(echo 'Hello World' | tr 'A-Z' 'a-z')" "hello world" "tr lower"
zassert_eq "$(echo 'alice,30,admin' | cut -d, -f1)" "alice" "cut -f1"
zassert_eq "$(echo 'alice,30,admin' | cut -d, -f2)" "30"    "cut -f2"
zassert_eq "$(echo 'alice,30,admin' | cut -d, -f3)" "admin" "cut -f3"
zassert_eq "$({ echo banana; echo apple; echo cherry; echo banana; } | sort -u | head -2)" $'apple\nbanana' "sort -u + head"
ztest_run

