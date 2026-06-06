#!/usr/bin/env zshrs
# $funcstack / $functrace / $funcfiletrace — call stack introspection.
# Ported from Src/exec.c funcstack handling.

show_stack() {
    echo "  --- inside show_stack ---"
    echo "  \$funcstack: ${(@)funcstack}"
    echo "  depth: ${#funcstack[@]}"
    if [[ -n $LINENO ]]; then
        echo "  caller line: $LINENO"
    fi
    if (( ${+funcfiletrace} )); then
        echo "  funcfiletrace: ${(@)funcfiletrace}"
    fi
}

level3() {
    show_stack
}

level2() {
    level3
}

level1() {
    level2
}

echo "── triple-nested call ──"
level1

echo
echo "── single-level call ──"
show_stack

echo
echo "── show fn-history ──"
my_fn() {
    echo "in my_fn"
    echo "  zsh_lineno (current): $LINENO"
    echo "  shlvl: $SHLVL"
}
my_fn

echo
echo "── recursive depth limit (won't infinite) ──"
typeset -i CALL_DEPTH=0
recurse() {
    (( CALL_DEPTH++ ))
    if (( CALL_DEPTH > 10 )); then
        echo "  recursion limit (depth=$CALL_DEPTH)"
        return
    fi
    recurse
}
recurse
echo "max depth reached: $CALL_DEPTH"

echo
echo "── caller info pattern ──"
log_with_caller() {
    local msg=$1
    local caller=${funcstack[2]:-main}
    echo "[caller: $caller] $msg"
}
worker() {
    log_with_caller "worker did its thing"
}
worker

echo
echo "── self-referential ──"
self_info() {
    echo "  function: ${funcstack[1]}"
    echo "  args: $*"
    echo "  arity: $#"
}
self_info one two three

# === ztest assertions ===
zassert_eq "$CALL_DEPTH" 11 "recursion depth hit 11"
si=$(self_info one two three)
zassert_contains "$si" "function: self_info" "self_info knows its own name"
zassert_contains "$si" "args: one two three" "self_info shows args"
zassert_contains "$si" "arity: 3"            "self_info shows arity"
lwc=$(worker)
zassert_contains "$lwc" "[caller: worker]"   "log_with_caller sees caller"
zassert_contains "$lwc" "worker did its thing" "log_with_caller payload"
nested=$(level1)
zassert_contains "$nested" "show_stack"      "nested call has show_stack in stack"
zassert_contains "$nested" "level1"          "nested stack contains level1"
zassert_contains "$nested" "depth: 4"        "depth=4 in triple-nested"
ztest_run
