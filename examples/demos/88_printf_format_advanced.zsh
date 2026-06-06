#!/usr/bin/env zshrs
# printf format specifiers — full coverage.
# Ported from Src/builtin.c bin_print (and printf format dispatch).

echo "── integer formats ──"
printf "decimal: %d\n" 42
printf "octal:   %o\n" 64
printf "hex lo:  %x\n" 255
printf "hex up:  %X\n" 255
printf "char:    %c\n" 65

echo "── unsigned + width ──"
printf "padded: [%5d]\n" 42
printf "zero:   [%05d]\n" 42
printf "left:   [%-5d]\n" 42
printf "signed: [%+d]\n" 42

echo "── floating point ──"
printf "fixed:    %.4f\n" 3.14159265
printf "exp:      %.3e\n" 314.159
printf "general:  %g\n" 0.0001
printf "general:  %g\n" 100000.0

echo "── strings ──"
printf "literal:    %s\n" hello
printf "padded:     [%10s]\n" hi
printf "left:       [%-10s]\n" hi
printf "truncated:  [%.3s]\n" toolong

echo "── escapes ──"
printf 'tab\there\n'
printf 'newline\nbetween\n'
printf 'quoted\\\"\n'
printf 'backslash\\\\\n'

echo "── multiple args one fmt ──"
printf "%-8s %5d\n" alice 30 bob 25 carol 35 dave 40

echo "── repeating ──"
printf "%d " 1 2 3 4 5
echo

echo "── %b (interpret escapes in arg) ──"
printf "%b\n" 'with\ttab'

echo "── piped via xxd-ish hex dump ──"
for c in {65..70}; do
    printf "%d → 0x%02x → \\x$(printf %x $c)\n" $c $c
done

echo "── table output ──"
printf "%-10s %4s %10s\n" "Name" "Age" "Role"
printf "%-10s %4s %10s\n" "----" "---" "----"
printf "%-10s %4d %10s\n" alice 30 admin
printf "%-10s %4d %10s\n" bob 25 user
printf "%-10s %4d %10s\n" carol 35 guest

# === ztest assertions ===
zassert_eq "$(printf '%d' 42)"        "42"            "%d"
zassert_eq "$(printf '%o' 64)"        "100"           "%o"
zassert_eq "$(printf '%x' 255)"       "ff"            "%x"
zassert_eq "$(printf '%X' 255)"       "FF"            "%X"
zassert_eq "$(printf '[%5d]' 42)"     "[   42]"       "%5d width"
zassert_eq "$(printf '[%05d]' 42)"    "[00042]"       "%05d zero-pad"
zassert_eq "$(printf '[%-5d]' 42)"    "[42   ]"       "%-5d left-pad"
zassert_eq "$(printf '[%+d]' 42)"     "[+42]"         "%+d sign"
zassert_eq "$(printf '%.4f' 3.14159265)" "3.1416"     "%.4f rounded"
zassert_eq "$(printf '%.3e' 314.159)" "3.142e+02"     "%.3e exp"
zassert_eq "$(printf '%g' 0.0001)"    "0.0001"        "%g small"
zassert_eq "$(printf '%g' 100000.0)"  "100000"        "%g large"
zassert_eq "$(printf '%s' hello)"     "hello"         "%s"
zassert_eq "$(printf '[%10s]' hi)"    "[        hi]"  "%10s right-align"
zassert_eq "$(printf '[%-10s]' hi)"   "[hi        ]"  "%-10s left-align"
zassert_eq "$(printf '[%.3s]' toolong)" "[too]"       "%.3s truncate"
zassert_eq "$(printf '%b' 'with\ttab')" $'with\ttab'  "%b escape interpretation"
ztest_run

