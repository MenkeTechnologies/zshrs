#!/usr/bin/env zshrs
# Parameter expansion match / substitute flags — (M) (P) (V) (Q) (q) (g).
# Ported from zsh's subst.c paramsubst (Src/subst.c:2414+ huge dispatch
# parses the flag block between `${(...)` and the param name).

arr=(apple banana cherry banana apple grape)

echo "── (M):# match-keep ──"
# `${(M)arr:#pattern}` — keep only entries that match pattern.
print -l ${(M)arr:#a*}

echo "── :# match-drop (no flag) ──"
# `${arr:#pattern}` — drop entries that match.
print -l ${arr:#a*}

echo "── (P) name-reference indirection ──"
# Reads the value of the variable whose NAME is in the param.
varname=arr
echo "${(P)varname}"

echo "── (Q) dequote ──"
quoted='\"foo\" \\bar\\'
echo "raw   : $quoted"
echo "(Q)   : ${(Q)quoted}"

echo "── (q) quote for shell input ──"
unsafe="hello \"world\" \$x"
echo "raw   : $unsafe"
echo "(q)   : ${(q)unsafe}"
echo "(qq)  : ${(qq)unsafe}"

echo "── (V) make invisibles visible ──"
mixed=$'foo\nbar\tbaz'
echo "raw width=${#mixed}"
echo "(V)   : ${(V)mixed}"

# === ztest assertions ===
# (M):# keep matching — apple appears twice in arr
kept=( ${(M)arr:#a*} )
zassert_eq "${#kept[@]}" 2 "(M):# kept 2 apples"
zassert_eq "${kept[1]}"  "apple" "(M):# first match"
# :# drop matching
dropped=( ${arr:#a*} )
zassert_eq "${#dropped[@]}" 4 "':#' dropped a-prefix → 4 left"
zassert_eq "${dropped[1]}" "banana" "first non-a entry"
# (P) indirection
varname=arr
zassert_eq "${(P)varname}" "apple banana cherry banana apple grape" "(P) indirect read"
# raw length of $'foo\nbar\tbaz' = 11
zassert_eq "${#mixed}" 11 "raw width with newline+tab"
# quoted forms
unsafe='hello "world" $x'
qq=${(qq)unsafe}
expected_qq=\''hello "world" $x'\'
zassert_eq "$qq" "$expected_qq" "(qq) single-quoted form"
ztest_run
