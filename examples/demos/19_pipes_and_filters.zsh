#!/usr/bin/env zshrs
# Pipes and standard filters — sort, head, tail, tr, wc, cut.

echo "── line count ──"
printf "alpha\nbeta\ngamma\ndelta\nepsilon\n" | wc -l

echo "── sort + head ──"
printf "banana\napple\ncherry\nfig\ndate\n" | sort | head -3

echo "── sort reverse ──"
printf "1\n3\n2\n4\n0\n" | sort -n -r

echo "── unique ──"
printf "a\nb\na\nc\nb\na\n" | sort -u

echo "── case translate ──"
echo "Hello World" | tr 'a-z' 'A-Z'
echo "Hello World" | tr 'A-Z' 'a-z'

echo "── delete chars ──"
echo "h-e-l-l-o" | tr -d '-'

echo "── word count via wc ──"
echo "the quick brown fox jumps" | wc -w

echo "── grep filter ──"
printf "apple\nbanana\nape\ncherry\n" | grep '^a'

# === ztest assertions ===
zassert_contains "$(printf 'a\nb\nc\nd\ne\n' | wc -l)" "5"  "wc -l 5 lines"
zassert_eq "$(printf 'b\na\nc\n' | sort | head -1)" "a"     "sort + head -1"
zassert_eq "$(printf '1\n3\n2\n' | sort -n -r | head -1)" "3" "sort -nr top"
zassert_eq "$(printf 'a\nb\na\nc\nb\na\n' | sort -u | wc -l | tr -d ' ')" "3" "sort -u unique count"
zassert_eq "$(echo 'Hello World' | tr 'a-z' 'A-Z')" "HELLO WORLD" "upper-case tr"
zassert_eq "$(echo 'Hello World' | tr 'A-Z' 'a-z')" "hello world" "lower-case tr"
zassert_contains "$(echo 'the quick brown fox jumps' | wc -w)" "5" "wc -w 5 words"
zassert_eq "$(printf 'apple\nape\nbanana\n' | grep '^a' | wc -l | tr -d ' ')" "2" "grep ^a counts 2"
ztest_run
