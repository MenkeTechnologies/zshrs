#!/usr/bin/env zshrs
# Advanced brace expansion — nested, cross-product, sequence step, padding.
# Ported from zsh's Src/glob.c bracecomplete + Src/utils.c hasbraces.

echo "── nested groups ──"
echo {{1..3},{a..c}}

echo "── cross product ──"
echo {a,b,c}{1,2,3}

echo "── three-way cross product ──"
echo {x,y}{1,2}{p,q}

echo "── numeric range with step ──"
echo {0..20..5}
echo {100..50..-10}

echo "── zero-padded sequence ──"
echo {01..10}
echo {001..010}
echo {001..030..5}

echo "── letter range ──"
echo {a..e}
echo {A..E}
# zsh: backward letter range is reversed:
echo {e..a}

echo "── prefix + suffix ──"
echo file_{01..05}.txt
echo /tmp/dir_{a,b,c}/sub

echo "── combined with parameter expansion ──"
prefix=item
echo ${prefix}_{1..4}

echo "── escape to prevent expansion ──"
echo \{not,expanded\}
echo "{not,expanded}"   # quoted — also literal

echo "── use brace expansion to create files ──"
tmpdir=/tmp/zshrs_brace_$$
mkdir -p "$tmpdir"
touch "$tmpdir/file"_{a,b,c}{1,2}.log
echo "created:"
ls "$tmpdir"
rm -rf "$tmpdir"

echo "── range-iteration in a for loop ──"
for i in {1..3}; do
    for j in {a..c}; do
        echo "($i,$j)"
    done
done

echo "── sequence in array literal ──"
arr=(item_{01..05})
echo "n=${#arr[@]}"
print -l ${arr}

# === ztest assertions ===
zassert_eq "$(echo {{1..3},{a..c}})" "1 2 3 a b c"          "nested brace groups"
zassert_eq "$(echo {a,b,c}{1,2,3})"  "a1 a2 a3 b1 b2 b3 c1 c2 c3" "cross product"
zassert_eq "$(echo {0..20..5})"      "0 5 10 15 20"          "numeric range with step"
zassert_eq "$(echo {01..10})"        "01 02 03 04 05 06 07 08 09 10" "zero-padded sequence"
zassert_eq "$(echo {a..e})"          "a b c d e"             "letter range asc"
zassert_eq "$(echo {e..a})"          "e d c b a"             "letter range desc"
zassert_eq "$(echo file_{01..03}.txt)" "file_01.txt file_02.txt file_03.txt" "prefix + suffix"
zassert_eq "$(echo \{not,expanded\})" "{not,expanded}"        "escaped braces"
zassert_eq "$(echo "{not,expanded}")" "{not,expanded}"        "quoted braces"
arr2=(item_{01..05})
zassert_eq "${#arr2[@]}" 5            "brace seq into array"
zassert_eq "${arr2[1]}"  "item_01"    "first brace seq element"
zassert_eq "${arr2[5]}"  "item_05"    "last brace seq element"
prefix=item
zassert_eq "$(echo ${prefix}_{1..4})" "item_1 item_2 item_3 item_4" "param + brace"
ztest_run

