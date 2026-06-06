#!/usr/bin/env zshrs
# local + typeset modifier flags — -x export, -g global, -i int, -r ro.
# Ported from Src/builtin.c bin_typeset modifier dispatch.

echo "── local -x (export) ──"
fn_export() {
    local -x DEMO_EXPORTED=visible-to-children
    env | grep DEMO_EXPORTED | head -1
}
fn_export
echo "after: ${DEMO_EXPORTED:-not_exported_outside}"

echo "── local -g (global from inside fn) ──"
fn_set_global() {
    local -g GLOBAL_VAR=set_inside_fn
}
fn_set_global
echo "outside fn: GLOBAL_VAR=$GLOBAL_VAR"
unset GLOBAL_VAR

echo "── local -i (integer) ──"
fn_int() {
    local -i n
    n="3 + 4"   # auto-evaluated as arithmetic
    echo "n=$n (auto-eval)"
}
fn_int

echo "── local -r (readonly inside fn) ──"
fn_ro() {
    local -r CONST=42
    echo "CONST=$CONST"
    # CONST=99  # would error
}
fn_ro

echo "── local -a (array) ──"
fn_arr() {
    local -a items=(alpha beta gamma)
    echo "items=${items[@]}"
}
fn_arr
echo "outside: items=${items[@]:-unset}"

echo "── local -A (assoc) ──"
fn_assoc() {
    local -A m=(k1 v1 k2 v2)
    echo "k1=${m[k1]} k2=${m[k2]}"
}
fn_assoc

echo "── nested local scopes ──"
outer() {
    local x=outer_val
    inner() {
        local x=inner_val
        echo "inside inner: $x"
    }
    inner
    echo "after inner returns: $x"
}
outer

echo "── private vs local (zsh: same in -fc) ──"
fn_private() {
    private p=hidden  # zsh-specific
    echo "p=$p"
}
fn_private 2>/dev/null && echo "private works" || echo "private not supported (OK)"
echo "outside: p=${p:-cleared}"

# === ztest assertions ===
# local -i auto-evaluates arithmetic on assignment
fn_int_check() {
    local -i z
    z="3 + 4"
    echo $z
}
zassert_eq "$(fn_int_check)" "7"  "local -i auto-eval"
# local -a fresh
fn_arr_check() {
    local -a it=(x y z)
    echo "${it[*]}"
}
zassert_eq "$(fn_arr_check)" "x y z"  "local -a"
# local -A fresh
fn_assoc_check() {
    local -A m=(k1 v1)
    echo "${m[k1]}"
}
zassert_eq "$(fn_assoc_check)" "v1"  "local -A"
# nested local scopes
outer_check() {
    local x=outer_val
    inner_check() { local x=inner_val; echo "$x"; }
    inner_check
    echo "$x"
}
zassert_eq "$(outer_check)" "inner_val
outer_val"  "nested local scopes"
# local var doesn't leak to caller
items=""
fn_arr_check >/dev/null
zassert_eq "${items:-unset}" "unset"  "local -a does not leak"
ztest_run
