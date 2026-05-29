#!/usr/bin/env zshrs
# Extended glob — ^ ~ # (1+ rep) ## (alt-rep) (a|b) and recursive **.
# Ported from zsh's pattern.c (Src/pattern.c patcompile/patmatch).

setopt extended_glob

arr=(apple banana avocado almond cherry)

echo "── alternation (a|b) ──"
for x in $arr; do
    [[ $x == (apple|cherry) ]] && echo "match: $x"
done

echo "── negation ^pat ──"
for x in $arr; do
    [[ $x == ^a* ]] && echo "not-a: $x"
done

echo "── except ~ — match A AND NOT B ──"
for x in $arr; do
    [[ $x == a*~apple ]] && echo "a-prefix but not apple: $x"
done

echo "── # = 0+ rep, ## = 1+ rep ──"
s=foobaarrr
[[ $s == foob(a#)r* ]] && echo "match a*-rep"
[[ $s == foob(a##)r* ]] && echo "match a+-rep"

echo "── recursive ** glob ──"
tmpdir=/tmp/zshrs_recglob_$$
mkdir -p "$tmpdir/sub/deeper"
touch "$tmpdir/top.txt" "$tmpdir/sub/mid.txt" "$tmpdir/sub/deeper/bot.txt"
print -l $tmpdir/**/*.txt
rm -rf "$tmpdir"

echo "── filtering via :# ──"
files=(test.txt foo.log bar.txt main.zsh)
# Remove .txt files.
print -l ${files:#*.txt}

echo "── globbing in parameter substitution: ${arr:#pat} ──"
# Keep only those NOT matching a*.
print -l ${arr:#a*}

echo "── (M):# match-keep ──"
print -l ${(M)arr:#a*}
