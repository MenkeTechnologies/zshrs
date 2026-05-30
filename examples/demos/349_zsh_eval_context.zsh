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
echo "── $FUNCSTACK[] (call stack) ──"
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
echo "── $FUNCFILETRACE[] (file:line) ──"
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
echo "  shell pid (\$\$): $$"
echo "  parent pid (\$PPID): $PPID"

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
FUNCNEST=
unset FUNCNEST
echo "  FUNCNEST cleared"
