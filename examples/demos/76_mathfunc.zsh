#!/usr/bin/env zshrs
# zsh/mathfunc module — sqrt sin cos exp log etc.
# Ported from zsh's Src/Modules/mathfunc.c (math_string + math_func +
# mtab[]). Loaded via `zmodload zsh/mathfunc` (autoloaded on first use
# in many shells).

zmodload zsh/mathfunc 2>/dev/null || true

echo "── square root ──"
echo "sqrt(4)   = $((sqrt(4)))"
echo "sqrt(2)   = $((sqrt(2)))"
echo "sqrt(100) = $((sqrt(100)))"
echo "sqrt(0.25) = $((sqrt(0.25)))"

echo "── trig ──"
echo "sin(0)    = $((sin(0)))"
echo "cos(0)    = $((cos(0)))"
# Pi via 4 * atan(1):
pi="$((4 * atan(1)))"
echo "pi (4·atan1) = $pi"
echo "sin(pi/2) = $((sin(pi/2)))"
echo "cos(pi)   = $((cos(pi)))"

echo "── inverse trig ──"
echo "atan(1)·4 = $((atan(1) * 4))     # pi"
echo "asin(1)·2 = $((asin(1) * 2))     # pi"
echo "acos(0)·2 = $((acos(0) * 2))     # pi"

echo "── log / exp ──"
echo "exp(1)    = $((exp(1)))         # e"
e=$((exp(1)))
echo "log(e)    = $((log(e)))         # 1"
echo "log10(100) = $((log10(100)))     # 2"
echo "log10(1e6) = $((log10(1000000)))  # 6"

echo "── powers ──"
echo "2^10      = $(( 2 ** 10 ))"
echo "2^0.5     = $(( 2 ** 0.5 ))"
echo "pow(3,4)  = $(( 3 ** 4 ))"

echo "── ceil / floor / abs ──"
echo "ceil(2.3)  = $((ceil(2.3)))"
echo "floor(2.7) = $((floor(2.7)))"
echo "abs(-7)    = $((abs(-7)))"

echo "── hyperbolic ──"
echo "sinh(0) = $((sinh(0)))"
echo "cosh(0) = $((cosh(0)))"
echo "tanh(0) = $((tanh(0)))"

# === ztest assertions ===
zassert_near $((sqrt(4)))       2.0     0.0001  "sqrt(4)"
zassert_near $((sqrt(2)))       1.41421 0.0001  "sqrt(2)"
zassert_near $((sqrt(100)))     10.0    0.0001  "sqrt(100)"
zassert_near $((sin(0)))        0.0     0.0001  "sin(0)"
zassert_near $((cos(0)))        1.0     0.0001  "cos(0)"
pi=$((4 * atan(1)))
zassert_near "$pi"              3.14159 0.0001  "pi via 4·atan(1)"
zassert_near $((sin(pi/2)))     1.0     0.0001  "sin(pi/2)"
zassert_near $((cos(pi)))       -1.0    0.0001  "cos(pi)"
zassert_near $((exp(1)))        2.71828 0.0001  "exp(1) → e"
zassert_near $((log10(100)))    2.0     0.0001  "log10(100)"
zassert_eq   $((abs(-7)))       7       "abs(-7)"
zassert_near $((sinh(0)))       0.0     0.0001  "sinh(0)"
zassert_near $((cosh(0)))       1.0     0.0001  "cosh(0)"
zassert_near $((tanh(0)))       0.0     0.0001  "tanh(0)"
ztest_run

