#!/usr/bin/env zshrs
# Integer arithmetic — operators, bit ops, base conversions.
echo "── arithmetic ──"
echo "2 + 3 = $((2 + 3))"
echo "10 - 4 = $((10 - 4))"
echo "6 * 7 = $((6 * 7))"
echo "100 / 7 = $((100 / 7))"
echo "100 % 7 = $((100 % 7))"
echo "2 ** 10 = $((2 ** 10))"
echo "-5 * 3 = $((-5 * 3))"
echo "precedence: 1 + 2 * 3 = $((1 + 2 * 3))"
echo "parens: (1 + 2) * 3 = $(( (1 + 2) * 3 ))"

echo "── bit ops ──"
echo "0xFF & 0x0F = $(( 0xFF & 0x0F ))"
echo "0xF0 | 0x0F = $(( 0xF0 | 0x0F ))"
echo "0xFF ^ 0xAA = $(( 0xFF ^ 0xAA ))"
echo "1 << 4 = $(( 1 << 4 ))"
echo "256 >> 2 = $(( 256 >> 2 ))"

echo "── base literals ──"
echo "hex 0xff = $(( 0xff ))"
echo "octal 0755 = $(( 0755 ))"
echo "binary 2#1010 = $(( 2#1010 ))"
