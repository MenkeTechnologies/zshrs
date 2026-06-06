#!/usr/bin/env zshrs
# print -aC N — array print in N columns.
# Ported from zsh's Src/builtin.c bin_print (the -a -C column-formatting path).

fruits=(apple banana cherry date elderberry fig grape honeydew
        kiwi lemon mango nectarine orange papaya quince raspberry)

echo "── 3 columns ──"
print -aC 3 $fruits

echo "── 4 columns ──"
print -aC 4 $fruits

echo "── 5 columns ──"
print -aC 5 $fruits

echo "── 1 column (single per-line) ──"
print -aC 1 alpha beta gamma delta

echo "── print -l (always one per line, regardless of width) ──"
print -l $fruits[1,5]

echo "── print -r raw (no escape interpretation) ──"
print -r 'literal\n\t$pwd'

echo "── print -n (no trailing newline) ──"
print -n "no newline → "
print "still on same line"

echo "── print -z push onto editor buffer (skip on non-interactive) ──"
# print -z would push but in -c mode there's no editor.
echo "(print -z skipped in -c mode)"

echo "── print -u N print to file descriptor N ──"
print -u 2 "this goes to stderr"

echo "── concatenate arrays then column-print ──"
combined=( ${fruits[1,8]} ${fruits[9,16]} )
print -aC 4 $combined

# === ztest assertions ===
zassert_eq "${#fruits[@]}"     16        "fruit array length"
zassert_eq "${fruits[1]}"      "apple"   "first fruit"
zassert_eq "${fruits[16]}"     "raspberry" "last fruit"
zassert_eq "${fruits[-1]}"     "raspberry" "negative index last"
zassert_eq "${#combined[@]}"   16        "concatenated array length"
out3=$(print -aC 3 $fruits)
zassert_contains "$out3" "apple"     "3-col output contains apple"
zassert_contains "$out3" "raspberry" "3-col output contains raspberry"
out1=$(print -aC 1 alpha beta gamma delta)
zassert_eq "$out1" $'alpha\nbeta\ngamma\ndelta' "1-col print -aC stacks one per line"
outl=$(print -l ${fruits[1,5]})
zassert_eq "$outl" $'apple\nbanana\ncherry\ndate\nelderberry' "print -l one per line"
outr=$(print -r 'literal\n\t$pwd')
zassert_eq "$outr" 'literal\n\t$pwd' "print -r preserves backslashes literally"
ztest_run

