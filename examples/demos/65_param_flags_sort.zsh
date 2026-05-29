#!/usr/bin/env zshrs
# Sort + unique parameter flags — (o) (O) (u) (n) (i).
# Ported from zsh's subst.c paramsubst (sort_flags branch).

arr=(banana apple cherry banana date apple Banana APPLE)

echo "── (o) ascending sort ──"
print -l ${(o)arr}

echo "── (O) descending sort ──"
print -l ${(O)arr}

echo "── (u) unique (preserves order) ──"
print -l ${(u)arr}

echo "── (ou) sort + unique ──"
print -l ${(ou)arr}

echo "── (oi) case-insensitive sort ──"
print -l ${(oi)arr}

echo "── (n) numeric sort ──"
nums=(10 2 33 1 100 50)
print -l ${(n)nums}

echo "── (On) descending numeric ──"
print -l ${(On)nums}

echo "── reverse a list ──"
src=(1 2 3 4 5)
print -l ${(Oa)src}    # (a) is array-position-reverse (Oa flips order)

echo "── pipeline: sort | unique on inline-built array ──"
words=(quick brown fox jumps over the lazy dog the quick brown fox)
print -l ${(ou)words}
