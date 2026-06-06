#!/usr/bin/env zshrs
# xtrace (set -x) demonstration — see what's executing.
# Ported from Src/builtin.c bin_set + PS4 expansion.

echo "── plain (no trace) ──"
x=5
y=10
echo "x+y=$((x+y))"

echo
echo "── trace with set -x ──"
{
    set -x
    a=1
    b=2
    c=$((a + b))
    echo "result=$c"
    set +x
} 2>&1 | head -10

echo
echo "── custom PS4 prompt ──"
{
    PS4='+[${FUNCNAME[0]:-main}:$LINENO] '
    set -x
    f1() {
        local v=42
        echo "in f1: v=$v"
    }
    f1
    set +x
} 2>&1 | head -10

echo
echo "── trace just one function ──"
trace_fn() {
    setopt local_options
    setopt xtrace
    echo "inside traced fn"
    local x=hello
    echo "x=$x"
}
trace_fn 2>&1 | head -5

echo "outside fn: not traced"

echo
echo "── trace with depth indication ──"
{
    PS4='${(l:$((SHLVL+1))::+:)} '
    set -x
    inner() {
        echo "in inner"
    }
    outer() {
        echo "in outer"
        inner
    }
    outer
    set +x
} 2>&1 | head -10

echo
echo "── conditional tracing via env var ──"
DEBUG=1
debug_fn() {
    if (( DEBUG )); then set -x; fi
    echo "step 1"
    local x=5
    echo "step 2"
    if (( DEBUG )); then set +x; fi
}
debug_fn 2>&1 | head -8

# === ztest assertions ===
zassert_eq "$x" 5  "outer x retained"
zassert_eq "$y" 10 "outer y retained"
zassert_eq $((x+y)) 15 "plain arithmetic"
zassert_eq "$DEBUG" 1 "DEBUG flag set"
# capture trace output and look for xtrace marker
trace_out=$({ set -x; a=11; b=22; c=$((a+b)); set +x; } 2>&1)
zassert_contains "$trace_out" "+" "trace has '+' prefix"
ztest_run
