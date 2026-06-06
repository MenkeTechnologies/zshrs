#!/usr/bin/env zshrs
# Advanced associative-array operations — (@kv) pairing, key-existence
# guard, copy, count, sort by value.
# Ported from zsh's params.c (PM_HASHED dispatch + getarg key matching).

typeset -A inv
inv[apple]=10
inv[banana]=5
inv[cherry]=20
inv[date]=8

echo "── size ──"
echo "${#inv[@]} items"

echo "── key existence (zsh idiom) ──"
for k in apple kiwi banana mango; do
    if [[ -n ${inv[$k]+x} ]]; then
        echo "$k: present (${inv[$k]})"
    else
        echo "$k: absent"
    fi
done

echo "── keys / values ──"
echo "keys: ${(ok)inv}"     # (ok) = sort-by-key
echo "vals: ${(o)inv}"      # implicit value iteration sorted

echo "── (@kv) k/v pairs ──"
for k v in "${(@kv)inv}"; do
    echo "  $k → $v"
done | sort

echo "── copy assoc to another ──"
typeset -A snapshot
for k v in "${(@kv)inv}"; do
    snapshot[$k]=$v
done
echo "snapshot size: ${#snapshot[@]}"
echo "snapshot[banana]: ${snapshot[banana]}"

echo "── sort entries by value (asc) ──"
typeset -a pairs
for k v in "${(@kv)inv}"; do
    pairs+=("$(printf '%05d %s' $v $k)")
done
for p in ${(o)pairs}; do
    set -- $=p
    echo "  $1 $2"
done

echo "── top-2 by value (desc) ──"
n=0
for p in ${(O)pairs}; do
    set -- $=p
    echo "  $1 $2"
    (( ++n >= 2 )) && break
done

echo "── delete a key ──"
unset 'inv[date]'
echo "after unset date: ${#inv[@]} items"
echo "remaining keys: ${(ok)inv}"

echo "── clear all ──"
inv=()
echo "after clear: ${#inv[@]}"

# === ztest assertions ===
typeset -A v
v[apple]=10
v[banana]=5
v[cherry]=20
v[date]=8
zassert_eq "${#v[@]}"  4    "4 keys in v"
zassert_eq "${v[apple]}"  10 "v[apple]=10"
zassert_eq "${v[banana]}" 5  "v[banana]=5"
# key existence idiom
[[ -n ${v[apple]+x} ]] && zassert_ok 1 "+x exists for apple"  || zassert_ok 0 "+x exists for apple"
[[ -n ${v[mango]+x} ]] && zassert_ok 0 "+x absent for mango"  || zassert_ok 1 "+x absent for mango"
# sorted keys
zassert_eq "${(ok)v}" "apple banana cherry date" "(ok) keys sorted"
# delete
unset 'v[date]'
zassert_eq "${#v[@]}"  3    "3 keys after unset date"
# clear
v=()
zassert_eq "${#v[@]}"  0    "0 keys after clear"
zassert_eq "${#snapshot[@]}"  4   "snapshot copy preserved 4 keys"
zassert_eq "${snapshot[banana]}"  5 "snapshot[banana]"
ztest_run
