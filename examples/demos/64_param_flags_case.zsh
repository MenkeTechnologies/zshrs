#!/usr/bin/env zshrs
# Case-mapping parameter flags — (L) (U) (C) + the :u :l modifiers.
# Ported from zsh's subst.c paramsubst case branch.

arr=(Hello World ZSH "Mixed Case")
s="The Quick Brown Fox"

echo "── (L) lowercase ──"
print -l ${(L)arr}

echo "── (U) uppercase ──"
print -l ${(U)arr}

echo "── (C) capitalize each word ──"
echo "${(C)s}"

echo "── :u :l modifiers (scalar) ──"
echo "  '$s'"
echo "  :u → '${s:u}'"
echo "  :l → '${s:l}'"

echo "── chained (L) + (j) ──"
echo "${(j:-:)${(L)arr}}"

echo "── case on first letter only via slice ──"
words=("hello" "world" "shell")
for w in $words; do
    first=${w[1]:u}
    rest=${w[2,-1]}
    echo "${first}${rest}"
done

echo "── (C) on assoc keys ──"
typeset -A m=( alpha 1 beta 2 gamma 3 )
for k in ${(k)m}; do
    echo "${(C)k}: ${m[$k]}"
done | sort

# === ztest assertions ===
larr=( ${(L)arr} )
zassert_eq "${larr[1]}"  "hello"     "(L) lowercase first elt"
zassert_eq "${larr[3]}"  "zsh"       "(L) lowercase ZSH"
zassert_eq "${larr[4]}"  "mixed case" "(L) lowercase 'Mixed Case'"
uarr=( ${(U)arr} )
zassert_eq "${uarr[1]}"  "HELLO"     "(U) uppercase first elt"
zassert_eq "${uarr[3]}"  "ZSH"       "(U) uppercase ZSH"
zassert_eq "${(C)s}"     "The Quick Brown Fox" "(C) capitalize each word"
zassert_eq "${s:u}"      "THE QUICK BROWN FOX" ":u modifier"
zassert_eq "${s:l}"      "the quick brown fox" ":l modifier"
word="alpha"
zassert_eq "${(C)word}" "Alpha" "(C) on single word"
ztest_run
