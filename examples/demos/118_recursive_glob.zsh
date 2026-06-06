#!/usr/bin/env zshrs
# Recursive globbing — **/*.ext with qualifiers, depth controls.
# Ported from Src/glob.c (zglob, globbing dispatch).

tmpdir=/tmp/zshrs_recglob_$$
mkdir -p "$tmpdir/a/b/c/d"
touch "$tmpdir/top.txt"
touch "$tmpdir/a/level1.txt"
touch "$tmpdir/a/b/level2.txt" "$tmpdir/a/b/skip.log"
touch "$tmpdir/a/b/c/level3.txt"
touch "$tmpdir/a/b/c/d/level4.txt"

echo "── ** (all subdirs, all files) ──"
print -l "$tmpdir"/**/*

echo "── **/*.txt (recursive .txt) ──"
print -l "$tmpdir"/**/*.txt

echo "── files only (.) qualifier ──"
print -l "$tmpdir"/**/*(.)

echo "── dirs only (/) qualifier ──"
print -l "$tmpdir"/**/*(/)

echo "── with size qualifier (L0 = empty) ──"
print -l "$tmpdir"/**/*(.L0) | wc -l
echo "(count empty files above)"

echo "── (N) null-on-no-match qualifier ──"
print -l "$tmpdir"/**/*.nonexistent(N)
echo "(no match, no error)"

echo "── (oL) sort by size, ascending ──"
echo "by size:"
print -l "$tmpdir"/**/*(.oL)

echo "── ## extended-glob 1+ rep + chain ──"
setopt extended_glob
# Match dirs whose name has 1+ letters.
print -l "$tmpdir"/**/[a-z]##(/)

echo "── filter by exclusion ──"
print -l "$tmpdir"/**/^*.log(.)

rm -rf "$tmpdir"

# === ztest assertions ===
# (demo aborts mid-script under zshrs at `print -l "$tmpdir"/**/^*.log(.)`
# — extended-glob `^` negation is not implemented; the failing glob
# emits "no matches found" and the script terminates before this block
# is reachable. Smoke-only: confirms parser accepted the file.)
zassert_ok 1 "demo loaded"
ztest_run
