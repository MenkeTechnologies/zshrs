#!/usr/bin/env zshrs
# print builtin advanced flags — -l -a -c -N -f -r -n -R.
# Ported from Src/builtin.c bin_print.

arr=(alpha beta gamma delta epsilon fig grape honeydew)

echo "── -l one per line ──"
print -l "${arr[@]}"

echo "── -aC N columnar ──"
print -aC 3 "${arr[@]}"

echo "── -c columns from a list ──"
print -aC 4 "${arr[@]}"

echo "── -N NUL-separated (suitable for xargs -0) ──"
print -N "${arr[@]}" | tr '\0' '|'
echo

echo "── -n no trailing newline ──"
print -n "no newline after this →"; echo " continued"

echo "── -r raw (no escape interpretation) ──"
print -r 'literal\ttab and \n newline'
print 'with escapes\t<-tab\n<-newline'

echo "── -f format (printf-style) ──"
print -f "%-10s %d\n" alice 30 bob 25 carol 35

echo "── -P prompt expansion ──"
print -P "%n@%M %~"

echo "── -R like -r (compat) ──"
print -R 'has\\backslash'

echo "── multiple flags combined ──"
print -lr 'raw\\one' 'raw\\two' 'raw\\three'

echo "── -u N print to fd ──"
print -u 2 "this goes to stderr"

echo "── -e (enable escapes — default) ──"
print -e 'tab\there'

echo "── -E (explicit no-escape) ──"
print -E 'tab\there'

echo "── -c columns honors widest entry ──"
mixed=(short medium longer "longest-of-all")
print -aC 2 "${mixed[@]}"
