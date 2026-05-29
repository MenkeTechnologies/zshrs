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
