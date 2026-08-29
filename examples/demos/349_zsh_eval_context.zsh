#!/usr/bin/env zshrs
# $ZSH_EVAL_CONTEXT + $FUNCNEST + $LASTWIDGET + $ZSH_SUBSHELL.
# Ports Src/init.c (eval_context tracking) + Src/exec.c (subshell counter).

echo "── ZSH_EVAL_CONTEXT (top level) ──"
echo "  current: ${ZSH_EVAL_CONTEXT:-N/A}"
echo "  expected at top: 'toplevel' or 'file'"

echo
echo "── inside function ──"
my_fn() {
    echo "  in my_fn:  context = ${ZSH_EVAL_CONTEXT:-N/A}"
    nested_fn() {
        echo "    in nested_fn: context = ${ZSH_EVAL_CONTEXT:-N/A}"
    }
    nested_fn
}
my_fn

echo
echo "── inside subshell ──"
(
    echo "  in (...): context = ${ZSH_EVAL_CONTEXT:-N/A}"
    echo "  ZSH_SUBSHELL = ${ZSH_SUBSHELL:-N/A} (1 = first level)"
    (
        echo "  nested (((): context = ${ZSH_EVAL_CONTEXT:-N/A}"
        echo "  ZSH_SUBSHELL = ${ZSH_SUBSHELL:-N/A}"
    )
)

echo
echo "── inside cmd-sub ──"
result=$(
    echo "context: ${ZSH_EVAL_CONTEXT:-N/A}"
    echo "subshell: ${ZSH_SUBSHELL:-N/A}"
)
echo "$result" | sed 's/^/  /'

echo
echo "── eval ──"
eval '
echo "  in eval: context = ${ZSH_EVAL_CONTEXT:-N/A}"
'

echo
echo "── FUNCNEST limit ──"
typeset -gi FUNCNEST=20
echo "  FUNCNEST = $FUNCNEST"
echo "  controls max recursion depth (default unlimited / large)"

recurse_count() {
    local depth=$1
    if (( depth >= 10 )); then
        echo "  reached depth $depth"
        return
    fi
    recurse_count $((depth + 1))
}
recurse_count 0

# Reset FUNCNEST higher.
FUNCNEST=1000
echo
echo "  reset FUNCNEST=$FUNCNEST"

echo
echo "── $0 inside function ──"
echo "  top-level \$0: $0"
fn_dump_0() {
    echo "  in fn:      \$0 = $0"
    echo "             argv0 = ${argv0:-N/A}"
}
fn_dump_0
echo "  argv-0 also accessible via shift to '\$0'"

echo
echo '── $FUNCSTACK[] (call stack) ──'
outer() {
    middle
}
middle() {
    inner
}
inner() {
    echo "  function stack:"
    local i
    for ((i=1; i<=${#funcstack}; i++)); do
        printf "    [%d] %s\n" $i "${funcstack[i]}"
    done
}
outer

echo
echo '── $FUNCFILETRACE[] (file:line) ──'
trace_fn() {
    echo "  funcfiletrace:"
    local i
    for ((i=1; i<=${#funcfiletrace}; i++)); do
        printf "    [%d] %s\n" $i "${funcfiletrace[i]}"
    done
}
trace_fn

echo
echo "── $LINENO ──"
echo "  current line: $LINENO"
echo "  next line:    next-line"
my_line_fn() {
    echo "  in fn at LINENO: $LINENO"
}
my_line_fn

echo
echo "── ZSH_NAME / ZSH_VERSION / ZSH_PATCHLEVEL ──"
echo "  ZSH_NAME:       ${ZSH_NAME:-N/A}"
echo "  ZSH_VERSION:    ${ZSH_VERSION:-N/A}"
echo "  ZSH_PATCHLEVEL: ${ZSH_PATCHLEVEL:-N/A}"

echo
echo "── PIPESTATUS ──"
true | true | false
echo "  pipestatus: ${pipestatus[@]}"
echo "  PIPESTATUS: ${PIPESTATUS[@]}"

true | true | true
echo "  all-true pipestatus: ${pipestatus[@]}"

echo
echo "── \$? exit status chain ──"
false
echo "  after false: \$? = $?"
true
echo "  after true:  \$? = $?"
(exit 42)
echo "  after exit 42: \$? = $?"

echo
echo "── \$$ vs \$\$ ──"
# Print only that the pids exist — the numbers differ on every run.
echo "  shell pid (\$\$) set: $(( $$ > 0 ))"
echo "  parent pid (\$PPID) set: $(( PPID > 0 ))"

echo
echo "── eval context flags ──"
echo "  possible eval_context values:"
echo "    file       — sourced script"
echo "    toplevel   — interactive line"
echo "    cmdsubst   — inside \$(...)"
echo "    eval       — inside eval"
echo "    shfunc     — inside function call"
echo "    loadautofunc — autoloaded function"
echo "    trap       — inside trap handler"

echo
echo "── reset ──"
# NB: `FUNCNEST=` is an empty value that evaluates to 0, and `unset FUNCNEST`
# does NOT restore the built-in default — the limit stays 0, which then
# rejects every subsequent function call. Restore the default explicitly.
FUNCNEST=500
echo "  FUNCNEST restored to $FUNCNEST (zsh default)"

# === ztest assertions ===
zassert_eq "$ZSH_NAME"      "zsh"           "ZSH_NAME"
zassert_match '^5\.'        "$ZSH_VERSION"  "ZSH_VERSION starts with 5."
# ZSH_SUBSHELL is "0" at top level (counter, not truthy flag)
zassert_eq   "$ZSH_SUBSHELL" "0" "ZSH_SUBSHELL=0 at top"
zassert_eq "$ZSH_EVAL_CONTEXT" "toplevel" "ZSH_EVAL_CONTEXT at script top level"
# funcstack inside nested calls: 3 frames
capture_stack() {
    typeset -ga STACK_SNAPSHOT
    STACK_SNAPSHOT=("${funcstack[@]}")
}
o1() { o2; }
o2() { o3; }
o3() { capture_stack; }
o1
zassert_eq "${STACK_SNAPSHOT[1]}" "capture_stack" "funcstack[1] = innermost"
zassert_eq "${STACK_SNAPSHOT[2]}" "o3"            "funcstack[2]"
zassert_eq "${STACK_SNAPSHOT[3]}" "o2"            "funcstack[3]"
zassert_eq "${STACK_SNAPSHOT[4]}" "o1"            "funcstack[4]"
# pipestatus
true | true | false
zassert_eq "${pipestatus[*]}" "0 0 1" "pipestatus false at end"
true | true | true
zassert_eq "${pipestatus[*]}" "0 0 0" "pipestatus all true"
# Exit chain via $?
false; zassert_eq "$?" "1"  "false → 1"
true;  zassert_eq "$?" "0"  "true → 0"
(exit 42); zassert_eq "$?" "42" "subshell exit 42"
ztest_run
