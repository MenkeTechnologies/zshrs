#!/usr/bin/env zshrs
# Anonymous functions with args + local scope.
# Ported from zsh's Src/exec.c is_anonymous_function_name + runshfunc.

echo "── basic anon fn with args ──"
() { echo "args: $*"; } alpha beta gamma

echo "── anon fn local scope ──"
() {
    local secret="locked"
    echo "inside: secret=$secret"
}
echo "outside: secret=${secret:-unset}"

echo "── return-via-stdout (via named fn helper) ──"
square_fn() { echo $(( $1 * $1 )); }
square=$(square_fn 7)
echo "7² = $square"

echo "── side-effect anon fn for setup ──"
typeset -A config
() {
    config[host]=localhost
    config[port]=8080
    config[user]=admin
}
echo "config: host=${config[host]} port=${config[port]}"

echo "── parameterized config builder ──"
config2() {
    typeset -A cfg
    () {
        cfg[name]=$1
        cfg[value]=$2
    } "$@"
    for k v in "${(@kv)cfg}"; do
        echo "  $k = $v"
    done
}
config2 mykey myval

echo "── chain of anon fns ──"
result=()
() { result+=("alpha-${1}") } step1
() { result+=("beta-${1}") } step2
() { result+=("gamma-${1}") } step3
print -l ${result[@]}

echo "── anon fn for scoped option flip ──"
() {
    setopt local_options
    setopt no_glob
    echo "inside: glob is off, * is literal: *"
}
echo "outside: glob restored, * expands"

echo "── recursive anon (immediate) ──"
() {
    local n=$1
    if (( n <= 1 )); then
        echo "step $n"
    else
        echo "step $n"
        () { echo "inner step $1" } $((n-1))
    fi
} 3

# === ztest assertions ===
zassert_eq "$(() { echo "args: $*"; } a b c)"  "args: a b c"   "anon fn with args"
zassert_eq "$(() { echo "n=$#"; } x y z w)"     "n=4"           "anon fn $# count"
# anon fn local scope must not leak
unset leakvar
() { local leakvar=inside; } >/dev/null
zassert_eq "${leakvar:-unset}" "unset" "anon fn local does not leak"
# anon fn with closure-style access to outer var
outer=hello
zassert_eq "$(() { echo $outer; })" "hello" "anon fn sees outer var"
# chain of anon fns over result array
res=()
() { res+=("a"); } >/dev/null
() { res+=("b"); } >/dev/null
() { res+=("c"); } >/dev/null
zassert_eq "${(j: :)res}" "a b c" "anon fn array append chain"
# square_fn captures via $()
zassert_eq "$(square_fn 7)"   "49"  "square 7"
zassert_eq "$(square_fn 12)"  "144" "square 12"
ztest_run

