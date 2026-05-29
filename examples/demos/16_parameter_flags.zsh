#!/usr/bin/env zshrs
# Zsh parameter expansion flags — (o), (O), (u), (L), (U), (k), (v).
arr=(banana apple cherry banana date apple)

echo "── (o) sorted ascending ──"
print -l ${(o)arr}

echo "── (O) sorted descending ──"
print -l ${(O)arr}

echo "── (u) unique (preserve first-seen order) ──"
print -l ${(u)arr}

echo "── (L) lowercase ──"
mixed=(Hello WORLD MixEd)
print -l ${(L)mixed}

echo "── (U) uppercase ──"
print -l ${(U)mixed}

echo "── (k) hash keys / (v) hash values ──"
typeset -A m=( one 1 two 2 three 3 )
print -l ${(k)m} | sort
echo "---"
print -l ${(v)m} | sort
