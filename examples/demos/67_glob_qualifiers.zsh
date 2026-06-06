#!/usr/bin/env zshrs
# File-type glob qualifiers — (.) (/) (@) and ordering (om/oL/od).
# Ported from zsh's glob.c qualifier dispatch + Src/utils.c stat helpers.

tmpdir=/tmp/zshrs_qual_$$
mkdir -p "$tmpdir/sub1" "$tmpdir/sub2"
trap "rm -rf $tmpdir" EXIT
cd "$tmpdir"

# Set distinct sizes for ordering demos.
echo "big content here yes" > big.txt
echo "tiny" > small.txt
echo "med size data" > medium.txt
touch zero.txt
mkdir empty_dir

echo "── (.) plain files ──"
print -l *(.)

echo "── (/) directories ──"
print -l *(/)

echo "── empty file (.L0) — exactly 0 bytes ──"
print -l *(.L0)

echo "── (oL) sort by size, ascending ──"
print -l *(.oL)

echo "── (OL) sort by size, descending ──"
print -l *(.OL)

echo "── (on) sort by name, ascending (default) ──"
print -l *(.on)

echo "── (om) sort by mtime (descending = newest first) ──"
sleep 1
touch zero.txt   # touch updates mtime
print -l *(.om)

echo "── glob NEG-match: ^pattern needs extended_glob ──"
setopt extended_glob
print -l *.txt~big.*    # all .txt except big.*

echo "── (#qN) N qualifier: null match instead of error ──"
print -l nonexistent_*(N)
echo "above returned without error"

# === ztest assertions ===
files=( *(.) )
zassert_eq "${#files[@]}"  4    "(.) — 4 plain files"
zassert_contains "${files[*]}" "big.txt"    "(.) includes big.txt"
zassert_contains "${files[*]}" "zero.txt"   "(.) includes zero.txt"
dirs=( *(/) )
zassert_eq "${#dirs[@]}"   3    "(/) — 3 directories"
zassert_contains "${dirs[*]}" "empty_dir" "(/) includes empty_dir"
empties=( *(.L0) )
zassert_eq "${#empties[@]}" 1   ".L0 — 1 zero-byte file"
zassert_eq "${empties[1]}" "zero.txt" ".L0 is zero.txt"
size_asc=( *(.oL) )
zassert_eq "${size_asc[1]}" "zero.txt" "(oL) smallest first"
zassert_eq "${size_asc[4]}" "big.txt"  "(oL) largest last"
size_desc=( *(.OL) )
zassert_eq "${size_desc[1]}" "big.txt" "(OL) largest first"
# null-match (N) yields empty array, no error
nope=( nonexistent_*(N) )
zassert_eq "${#nope[@]}"   0    "(N) null-match returns empty array"

cd /tmp
ztest_run
