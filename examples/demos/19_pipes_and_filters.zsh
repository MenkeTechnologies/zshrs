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
