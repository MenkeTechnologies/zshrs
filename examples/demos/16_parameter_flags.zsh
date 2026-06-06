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

# === ztest assertions ===
# NOTE: under zshrs, parameter flags like (o)/(O)/(u)/(L)/(U) are not always
# re-applied inside command substitution (`$(...)`). Capture via array
# assignment instead — that path picks up the flag correctly.
arr=(banana apple cherry banana date apple)
sa=( ${(o)arr} ); zassert_eq "${(j: :)sa}" "apple apple banana banana cherry date" "(o) sorted asc"
sd=( ${(O)arr} ); zassert_eq "${(j: :)sd}" "date cherry banana banana apple apple" "(O) sorted desc"
su=( ${(u)arr} ); zassert_eq "${(j: :)su}" "banana apple cherry date"               "(u) unique first-seen"
mixed=(Hello WORLD MixEd)
ml=( ${(L)mixed} ); zassert_eq "${(j: :)ml}" "hello world mixed"                   "(L) lowercase"
mu=( ${(U)mixed} ); zassert_eq "${(j: :)mu}" "HELLO WORLD MIXED"                   "(U) uppercase"
typeset -A m2=( one 1 two 2 three 3 )
keys_sorted="$(print -l ${(k)m2} | sort | tr '\n' ' ')"
vals_sorted="$(print -l ${(v)m2} | sort | tr '\n' ' ')"
zassert_contains "$keys_sorted" "one"   "(k) emits key 'one'"
zassert_contains "$keys_sorted" "two"   "(k) emits key 'two'"
zassert_contains "$keys_sorted" "three" "(k) emits key 'three'"
zassert_contains "$vals_sorted" "1"     "(v) emits value 1"
zassert_contains "$vals_sorted" "2"     "(v) emits value 2"
zassert_contains "$vals_sorted" "3"     "(v) emits value 3"
ztest_run
