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
