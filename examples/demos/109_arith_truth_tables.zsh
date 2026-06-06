#!/usr/bin/env zshrs
# Truth tables + boolean algebra via shell arithmetic.
# Ported from Src/math.c (boolean operator dispatch).

echo "── AND truth table ──"
echo "A B | A&B"
echo "----+----"
for a in 0 1; do
    for b in 0 1; do
        printf "%d %d |  %d\n" $a $b $(( a & b ))
    done
done

echo "── OR truth table ──"
for a in 0 1; do
    for b in 0 1; do
        printf "%d %d |  %d\n" $a $b $(( a | b ))
    done
done

echo "── XOR truth table ──"
for a in 0 1; do
    for b in 0 1; do
        printf "%d %d |  %d\n" $a $b $(( a ^ b ))
    done
done

echo "── NAND / NOR / XNOR derived ──"
echo "A B | NAND NOR XNOR"
for a in 0 1; do
    for b in 0 1; do
        nand=$(( ! (a & b) ))
        nor=$(( ! (a | b) ))
        xnor=$(( ! (a ^ b) ))
        printf "%d %d |  %2d  %2d  %2d\n" $a $b $nand $nor $xnor
    done
done

echo "── 3-bit binary counter ──"
for ((n = 0; n < 8; n++)); do
    b2=$(( (n >> 2) & 1 ))
    b1=$(( (n >> 1) & 1 ))
    b0=$(( n & 1 ))
    printf "%d → %d%d%d\n" $n $b2 $b1 $b0
done

echo "── full adder (a, b, cin) → (sum, cout) ──"
adder() {
    local a=$1 b=$2 cin=$3
    local sum=$(( a ^ b ^ cin ))
    local cout=$(( (a & b) | (cin & (a ^ b)) ))
    echo "$sum $cout"
}
echo "a b cin | sum cout"
for a in 0 1; do for b in 0 1; do for c in 0 1; do
    set -- $(adder $a $b $c)
    printf "%d %d %d   |  %d   %d\n" $a $b $c $1 $2
done; done; done

echo "── boolean identity check ──"
# De Morgan: !(a & b) == (!a | !b)
for a in 0 1; do
    for b in 0 1; do
        left=$(( ! (a & b) ))
        right=$(( ! a | ! b ))
        printf "a=%d b=%d  L=%d R=%d %s\n" $a $b $left $right \
            "$([[ $left -eq $right ]] && echo OK || echo FAIL)"
    done
done

# === ztest assertions ===
# AND: only (1,1) → 1
zassert_eq $(( 0 & 0 )) 0  "0 AND 0"
zassert_eq $(( 1 & 0 )) 0  "1 AND 0"
zassert_eq $(( 1 & 1 )) 1  "1 AND 1"
# OR: only (0,0) → 0
zassert_eq $(( 0 | 0 )) 0  "0 OR 0"
zassert_eq $(( 1 | 0 )) 1  "1 OR 0"
zassert_eq $(( 1 | 1 )) 1  "1 OR 1"
# XOR
zassert_eq $(( 1 ^ 1 )) 0  "1 XOR 1"
zassert_eq $(( 1 ^ 0 )) 1  "1 XOR 0"
# NAND/NOR/XNOR
zassert_eq $(( ! (1 & 1) )) 0  "NAND 1,1"
zassert_eq $(( ! (0 | 0) )) 1  "NOR 0,0"
zassert_eq $(( ! (1 ^ 0) )) 0  "XNOR 1,0"
# Full adder: 1+1+1 = sum=1, cout=1
sum=$(( 1 ^ 1 ^ 1 ))
cout=$(( (1 & 1) | (1 & (1 ^ 1)) ))
zassert_eq "$sum"  "1"  "adder sum 1+1+1"
zassert_eq "$cout" "1"  "adder cout 1+1+1"
# De Morgan: !(1 & 0) == (!1 | !0)
zassert_eq $(( ! (1 & 0) ))    $(( ! 1 | ! 0 ))  "De Morgan"
ztest_run
