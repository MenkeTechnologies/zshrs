#!/usr/bin/env zshrs
# typeset -i with base — display as base-N integer.
# Ported from zsh's params.c (PM_INTEGER + pm->base path).

echo "── typeset -i N base specifier ──"
typeset -i 16 hex=255
echo "hex (base 16): $hex"

typeset -i 2 bin=10
echo "bin (base  2): $bin"

typeset -i 8 oct=64
echo "oct (base  8): $oct"

typeset -i 10 dec=42
echo "dec (base 10): $dec"

echo "── numeric base in arithmetic literal ──"
echo "0xff      = $(( 0xff ))"
echo "2#1010    = $(( 2#1010 ))"
echo "16#ff     = $(( 16#ff ))"
echo "8#777     = $(( 8#777 ))"
echo "0755 (oct prefix) = $(( 0755 ))"

echo "── retype mid-flight via typeset -i N ──"
typeset -i 16 x=1023
echo "x stored as hex: $x"
(( x = 4095 ))
echo "x after assign 4095: $x"

echo "── separate base-typed views ──"
typeset -i 8 x8=4095
typeset -i 2 x2=10
echo "4095 in base 8: $x8"
echo "10 in base 2:   $x2"

echo "── floats (typeset -F / -E) ──"
typeset -F 3 pi=3.14159265
echo "pi (F 3): $pi"

typeset -E 4 ee=2.71828
echo "ee (E 4): $ee"

echo "── readonly numerics ──"
typeset -ri ROCONST=42
echo "ROCONST: $ROCONST"
