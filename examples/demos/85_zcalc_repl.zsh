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

# === ztest assertions ===
zassert_eq $(( 2 + 3 ))           5     "calc 2+3"
zassert_eq $(( 100 / 7 ))         14    "calc 100/7"
zassert_eq $(( 2 ** 10 ))         1024  "calc 2^10"
zassert_eq $(( 0xff ))            255   "calc 0xff"
zassert_eq $(( 0xFF + 1 ))        256   "calc 0xFF+1"
zassert_eq $(( 2#1010 ))          10    "calc binary"
zassert_eq $(( 8#777 ))           511   "calc octal"
zassert_eq $(( 16#deadbeef ))     3735928559 "calc 32-bit hex"
zassert_eq $(( 0xff & 0x0f ))     15    "bit and"
zassert_eq $(( ~0xff & 0xfff ))   3840  "bit not + and"
zassert_eq $(( 5 > 3 ? 1 : 0 ))   1     "ternary true"
zassert_eq $(( 5 < 3 ? 1 : 0 ))   0     "ternary false"
zassert_eq $(( 5 == 5 ? 100 : 200 ))   100   "ternary eq"
nn=0
(( nn = 5 ))
zassert_eq "$nn" 5    "assign within arith"
(( nn += 10 ))
zassert_eq "$nn" 15   "compound +="
(( nn *= 2 ))
zassert_eq "$nn" 30   "compound *="
zassert_eq $(( a = 5, b = 10, a + b )) 15 "comma-sequence result"
ztest_run

