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

# === ztest assertions ===
asc=( ${(o)arr} )
zassert_eq "${asc[1]}"  "apple"   "(o) first ascending"
zassert_eq "${asc[3]}"  "APPLE"   "(o) APPLE follows lowercase apples by codepoint"
zassert_eq "${asc[-1]}" "date"    "(o) last ascending"
desc=( ${(O)arr} )
zassert_eq "${desc[1]}" "date"    "(O) descending starts with date"
uniq=( ${(u)arr} )
zassert_eq "${#uniq[@]}" 6        "(u) unique count (case-sensitive)"
ou=( ${(ou)arr} )
zassert_eq "${#ou[@]}"   6        "(ou) sorted+unique count"
zassert_eq "${ou[1]}"    "apple"  "(ou) starts with apple"
nasc=( ${(n)nums} )
zassert_eq "${nasc[*]}"  "1 2 10 33 50 100" "(n) numeric ascending"
ndesc=( ${(On)nums} )
zassert_eq "${ndesc[*]}" "100 50 33 10 2 1" "(On) numeric descending"
oa=( ${(Oa)src} )
zassert_eq "${oa[*]}"    "5 4 3 2 1"        "(Oa) reverse array order"
wuniq=( ${(ou)words} )
zassert_eq "${#wuniq[@]}" 8       "(ou) 8 unique words"
ztest_run
