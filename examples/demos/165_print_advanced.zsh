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

# === ztest assertions ===
# -l one-per-line.
out_l="$(print -l alpha beta gamma)"
zassert_eq "$out_l" $'alpha\nbeta\ngamma'      "print -l one-per-line"
# -N NUL separator (replaced to | for inspection).
out_n="$(print -N a b c | tr '\0' '|')"
zassert_eq "$out_n" "a|b|c|"                   "print -N NUL-separated"
# -n suppresses trailing newline.
out_nn="$(print -n nonl)"
zassert_eq "$out_nn" "nonl"                    "print -n no trailing newline"
# -r raw — backslash sequences are literal.
out_r="$(print -r 'literal\ttab')"
zassert_eq "$out_r" 'literal\ttab'             "print -r literal"
# default print: \t expands.
out_def="$(print 'tab\there')"
zassert_contains "$out_def" $'\t'              "print default expands \\t"
# -f printf-style format.
out_f="$(print -f "%-10s %d\n" alice 30)"
zassert_contains "$out_f" "alice"              "print -f label"
zassert_contains "$out_f" "30"                 "print -f number"
# -R is like -r.
out_R="$(print -R 'has\\backslash')"
zassert_eq "$out_R" 'has\\backslash'           "print -R like -r"
ztest_run
