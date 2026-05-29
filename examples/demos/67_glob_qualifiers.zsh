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

cd /tmp
