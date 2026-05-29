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
