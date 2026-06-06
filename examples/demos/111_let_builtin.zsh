#!/usr/bin/env zshrs
# `let` arithmetic builtin — alternative to (( … )).
# Ported from Src/builtin.c bin_let.

echo "── basic let ──"
let x=5
let y=10
echo "x=$x y=$y"

echo "── let multi-expression ──"
let "a = 5" "b = 10" "c = a + b"
echo "a=$a b=$b c=$c"

echo "── compound assignment via let ──"
let n=100
let "n += 50"
let "n *= 2"
let "n -= 10"
echo "n after compound: $n"

echo "── let return code (0 if non-zero result, 1 if zero) ──"
let "1 + 1" ; echo "let '1+1' (non-zero): $?"
let "0" ; echo "let '0' (zero): $?"
let "1 - 1" ; echo "let '1-1' (zero): $?"

echo "── let in if condition ──"
let val=5
if let "val > 3"; then
    echo "val=$val > 3"
fi
if ! let "val > 100"; then
    echo "val=$val not > 100"
fi

echo "── let with parens ──"
let r='(5 + 3) * (10 - 2)'
echo "r = $r"

echo "── let with bit ops ──"
let mask=0xFF
let "shifted = mask << 4"
let "anded = mask & 0x0F"
printf "mask=0x%x shifted=0x%x anded=0x%x\n" $mask $shifted $anded

echo "── compared with (( )) ──"
(( x2 = 42 ))
let x3=42
echo "x2=$x2 x3=$x3 (equivalent)"

# === ztest assertions ===
# basic let
let lx=5
let ly=10
zassert_eq "$lx" "5"   "let lx=5"
zassert_eq "$ly" "10"  "let ly=10"
# multi-expression let
let "la = 5" "lb = 10" "lc = la + lb"
zassert_eq "$lc" "15"  "let multi result"
# compound assignment
let ln=100
let "ln += 50"
let "ln *= 2"
let "ln -= 10"
zassert_eq "$ln" "290"  "let compound chain"
# let exit code: nonzero result → 0; zero result → 1.
let "1 + 1" ; zassert_eq "$?" "0"  "let nonzero exit"
let "0"     ; zassert_eq "$?" "1"  "let zero exit"
let "1 - 1" ; zassert_eq "$?" "1"  "let subtract-zero exit"
# parenthesized
let lr='(5 + 3) * (10 - 2)'
zassert_eq "$lr" "64"  "let parens"
# bit ops
let lmask=0xFF
let "lsh = lmask << 4"
let "land = lmask & 0x0F"
# zshrs preserves the "16#" hex prefix in scalar form; numeric comparison still works.
zassert_eq "$lmask" "16#FF"  "let hex literal keeps base prefix"
zassert_eq "$lsh"   "4080"   "let shift (decimal — left operand promoted)"
zassert_eq "$land"  "16#F"   "let mask retains base prefix"
zassert_eq $(( lmask ))  "255"  "numeric value of hex-prefixed scalar"
zassert_eq $(( land ))   "15"   "numeric value of mask"
ztest_run
