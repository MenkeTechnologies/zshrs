#!/usr/bin/env zshrs
# Arithmetic edge cases — overflow, division, modulo, ternary nesting.
# Ported from Src/math.c mathevall.

echo "── integer overflow at i64 boundary ──"
echo "i64 max: $(( (1 << 62) + ((1 << 62) - 1) ))"  # 2^63 - 1
echo "+1: $(( ((1 << 62) + ((1 << 62) - 1)) + 1 ))"   # overflow
echo "neg min: $(( - (1 << 62) - (1 << 62) ))"

echo "── division by zero behavior ──"
err_div() { eval 'echo $(( 5 / 0 ))' 2>&1 | head -1; }
err_div

echo "── modulo with negatives ──"
echo "-7 % 3 = $(( -7 % 3 ))"
echo "7 % -3 = $(( 7 % -3 ))"
echo "-7 % -3 = $(( -7 % -3 ))"

echo "── ternary nesting ──"
echo "Right-assoc: $(( 1 ? 2 : 3 ? 4 : 5 ))"
echo "False then ternary: $(( 0 ? 1 : (5 < 10 ? 50 : 99) ))"

echo "── short-circuit && and || ──"
counter=0
inc() { (( counter++ )); echo 1; }
# Short-circuit: false && inc never increments.
(( 0 && $(inc) ))
echo "after 0 && inc: counter=$counter"
(( 1 || $(inc) ))
echo "after 1 || inc: counter=$counter (should still be 0)"

echo "── pre/post inc ordering ──"
i=5
echo "i++ = $((i++)) (returns OLD value)"
echo "after: i=$i"
i=5
echo "++i = $((++i)) (returns NEW value)"
echo "after: i=$i"

echo "── bit ops chain ──"
echo "(0xff & 0x0f) | 0x10 = $(( (0xff & 0x0f) | 0x10 ))"
echo "1<<8 = $(( 1 << 8 ))"
echo "1<<32 = $(( 1 << 32 ))"

echo "── parens and precedence ──"
echo "2+3*4 = $(( 2 + 3 * 4 ))"
echo "(2+3)*4 = $(( (2+3) * 4 ))"
echo "2**3**2 (right-assoc): $(( 2 ** 3 ** 2 ))"

echo "── comma operator ──"
echo "a=5, b=10, a+b: $(( a = 5, b = 10, a + b ))"
echo "after: a=$a b=$b"

echo "── float promotion ──"
echo "5 / 2 (int): $(( 5 / 2 ))"
echo "5.0 / 2 (float): $(( 5.0 / 2 ))"
echo "5 / 2.0 (float): $(( 5 / 2.0 ))"

# === ztest assertions ===
# Modulo with negatives (zsh: truncated-toward-zero)
zassert_eq $(( -7 % 3 ))   -1   "-7 % 3"
zassert_eq $(( 7 % -3 ))    1   "7 % -3"
zassert_eq $(( -7 % -3 ))  -1   "-7 % -3"
# Ternary precedence (right-assoc)
zassert_eq $(( 1 ? 2 : 3 ? 4 : 5 ))             2  "ternary right-assoc"
zassert_eq $(( 0 ? 1 : (5 < 10 ? 50 : 99) ))   50  "ternary nested"
# Pre/post increment
i=5
v_post=$((i++))
zassert_eq "$v_post" "5"  "i++ returns old"
zassert_eq "$i"      "6"  "i after i++"
i=5
v_pre=$((++i))
zassert_eq "$v_pre"  "6"  "++i returns new"
# Bit ops chain
zassert_eq $(( (0xff & 0x0f) | 0x10 ))   31   "bit-ops chain"
zassert_eq $(( 1 << 8 ))                256   "1<<8"
# Right-assoc **
zassert_eq $(( 2 ** 3 ** 2 ))           512   "right-assoc pow"
# Comma operator
zassert_eq $(( a = 5, b = 10, a + b ))   15   "comma op result"
zassert_eq "$a" "5"   "comma op left effect"
zassert_eq "$b" "10"  "comma op middle effect"
# Float division
zassert_near $(( 5.0 / 2 ))  2.5   0.0001  "float div"
ztest_run
