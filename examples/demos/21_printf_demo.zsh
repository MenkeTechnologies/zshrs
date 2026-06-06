#!/usr/bin/env zshrs
# printf — format specifiers.

echo "── strings ──"
printf "%s\n" "plain"
printf "[%10s]\n" "right"
printf "[%-10s]\n" "left"
printf "[%.5s]\n" "truncate-this-one"

echo "── integers ──"
printf "%d\n" 42
printf "%05d\n" 7
printf "%+d %+d\n" 10 -10
printf "%x %X\n" 255 255
printf "%o\n" 8

echo "── multiple in one fmt ──"
printf "%-10s %5d %8s\n" alice 42 admin
printf "%-10s %5d %8s\n" bob 100 user
printf "%-10s %5d %8s\n" carol 7 guest

echo "── escapes ──"
printf 'tab\there\n'
printf 'quote\\\"\n'

echo "── repeating fmt ──"
printf "%d " 1 2 3 4 5
echo

# === ztest assertions ===
zassert_eq "$(printf '%s' plain)"        "plain"          "plain %s"
zassert_eq "$(printf '[%10s]' right)"    "[     right]"   "right-pad %10s"
zassert_eq "$(printf '[%-10s]' left)"    "[left      ]"   "left-pad %-10s"
zassert_eq "$(printf '[%.5s]' truncate-this-one)" "[trunc]" "truncate %.5s"
zassert_eq "$(printf '%05d' 7)"          "00007"          "zero-pad %05d"
zassert_eq "$(printf '%+d %+d' 10 -10)"  "+10 -10"        "signed %+d"
zassert_eq "$(printf '%x %X' 255 255)"   "ff FF"          "hex %x/%X"
zassert_eq "$(printf '%o' 8)"            "10"             "octal %o"
zassert_eq "$(printf '%d ' 1 2 3 4 5)"   "1 2 3 4 5 "     "repeating fmt"
zassert_contains "$(printf 'tab\there')" $'\t'             "tab escape"
ztest_run
