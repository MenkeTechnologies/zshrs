#!/usr/bin/env zshrs
# Minimal calculator REPL — drives matheval through a deterministic
# input list (no interactive prompt — CI must run cleanly).
# Demonstrates zsh's $((...)) integration: variable assignment within
# expression, base literals, mathfunc-backed calls, multi-statement.

zmodload zsh/mathfunc 2>/dev/null || true

calc() {
    local expr=$1
    local result=$(( expr ))
    printf "%-30s = %s\n" "$expr" "$result"
}

echo "── basic arithmetic ──"
calc "2 + 3"
calc "10 - 4"
calc "6 * 7"
calc "100 / 7"
calc "100 % 7"

echo "── precedence ──"
calc "1 + 2 * 3"
calc "(1 + 2) * 3"
calc "2 ** 10"
calc "2 ** 0.5"

echo "── base literals ──"
calc "0xff"
calc "0xFF + 1"
calc "2#1010"
calc "8#777"
calc "16#deadbeef"

echo "── bit ops ──"
calc "0xff & 0x0f"
calc "0xf0 | 0x0f"
calc "0xff ^ 0xaa"
calc "1 << 8"
calc "256 >> 2"
calc "~0xff & 0xfff"

echo "── conditionals (ternary) ──"
calc "5 > 3 ? 1 : 0"
calc "5 < 3 ? 1 : 0"
calc "5 == 5 ? 100 : 200"

echo "── assignment WITHIN expression ──"
n=0
calc "n = 5"
echo "n is now: $n"
calc "n += 10"
echo "n is now: $n"
calc "n *= 2"
echo "n is now: $n"

echo "── ++/-- side effects ──"
i=0
calc "i++"
echo "i after post-inc: $i"
calc "++i"
echo "i after pre-inc: $i"

echo "── math functions ──"
calc "sqrt(16)"
calc "sin(0)"
calc "cos(0)"
calc "exp(0)"
calc "log(1)"
calc "atan(1) * 4"

echo "── multi-statement (comma) ──"
calc "a = 5, b = 10, a + b"
echo "a=$a b=$b"
