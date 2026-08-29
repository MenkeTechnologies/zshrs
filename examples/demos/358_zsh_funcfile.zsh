#!/usr/bin/env zshrs
# Function metadata — $funcfiletrace, $functrace, $funcsourcetrace, $funcstack.
# Ports Src/exec.c (func stack push/pop) + Src/init.c (trace assignments).

echo '── $funcstack[] at top level ──'
echo "  size: ${#funcstack}"
echo "  contents: ${funcstack[@]:-(empty)}"

echo
echo '── $funcstack[] inside function ──'
f1() {
    echo "  in f1:"
    echo "    funcstack size: ${#funcstack}"
    echo "    funcstack: ${funcstack[@]}"
    f2
}

f2() {
    echo "  in f2:"
    echo "    funcstack size: ${#funcstack}"
    echo "    funcstack: ${funcstack[@]}"
    f3
}

f3() {
    echo "  in f3:"
    echo "    funcstack size: ${#funcstack}"
    echo "    funcstack: ${funcstack[@]}"
}

f1

echo
echo '── $funcfiletrace[] (file:line where caller invoked) ──'
trace_dump() {
    echo "  in trace_dump:"
    echo "    funcfiletrace size: ${#funcfiletrace}"
    local i
    for ((i=1; i<=${#funcfiletrace}; i++)); do
        printf "    [%d] %s\n" $i "${funcfiletrace[i]}"
    done
}

caller_a() {
    trace_dump
}

caller_b() {
    caller_a
}

caller_b

echo
echo '── $functrace[] (caller function + line) ──'
trace_dump2() {
    echo "  in trace_dump2:"
    echo "    functrace size: ${#functrace}"
    local i
    for ((i=1; i<=${#functrace}; i++)); do
        printf "    [%d] %s\n" $i "${functrace[i]}"
    done
}

deep1() {
    deep2
}
deep2() {
    deep3
}
deep3() {
    trace_dump2
}

deep1

echo
echo '── $funcsourcetrace[] (file:line of function definition) ──'
dump_src_trace() {
    echo "  funcsourcetrace size: ${#funcsourcetrace}"
    local i
    for ((i=1; i<=${#funcsourcetrace}; i++)); do
        printf "    [%d] %s\n" $i "${funcsourcetrace[i]}"
    done
}

src_a() { src_b; }
src_b() { dump_src_trace; }

src_a

echo
echo "── recursive function with stack ──"
recursive_dump() {
    local depth=$1
    if (( depth <= 0 )); then return; fi
    echo "  depth=$depth, stack size=${#funcstack}, top=${funcstack[1]:-N/A}"
    recursive_dump $((depth - 1))
}

recursive_dump 4

echo
echo "── $ZSH_SCRIPT (script filename) ──"
echo "  ZSH_SCRIPT: ${ZSH_SCRIPT:-N/A}"
echo "  ZSH_ARGZERO: ${ZSH_ARGZERO:-N/A}"
echo "  \$0:        $0"

echo
echo "── inside an anonymous function ──"
() {
    echo "  in anon fn:"
    echo "    funcstack[1]: ${funcstack[1]:-(anon)}"
    echo "    funcstack: ${funcstack[@]}"
}

echo
echo "── via eval ──"
eval '
echo "  via eval:"
echo "  funcstack size: ${#funcstack}"
'

echo
echo "── via source ──"
tmpfile=$(mktemp)
cat > $tmpfile <<'EOF'
echo "  in sourced file:"
echo "    funcstack size: ${#funcstack}"
echo "    funcfiletrace size: ${#funcfiletrace}"
echo "    funcfiletrace: ${funcfiletrace[@]}"
EOF
source $tmpfile
command rm -f $tmpfile

echo
echo "── stack capture for error reporting ──"
report_error() {
    local msg=$1
    echo "  ERROR: $msg"
    echo "  call trace:"
    local i
    for ((i=1; i<=${#funcstack}; i++)); do
        printf "    [%d] %s   (from %s)\n" $i "${funcstack[i]}" "${funcfiletrace[i]:-(unknown)}"
    done
}

api_call() {
    db_query
}

db_query() {
    auth_check
}

auth_check() {
    report_error "auth token expired"
}

api_call

echo
echo "── reflection: list defined functions ──"
echo "  total functions defined: $(typeset -f + | wc -l)"

echo
echo "── $functions assoc — function body lookup ──"
my_introspect_fn() {
    echo "hello from introspection"
    local x=42
    echo "x = \$x"
}

echo "  body of my_introspect_fn:"
echo "${functions[my_introspect_fn]}" | sed 's/^/    /'

echo
echo "── unfunction all helpers ──"
unfunction f1 f2 f3 trace_dump caller_a caller_b 2>/dev/null
unfunction trace_dump2 deep1 deep2 deep3 2>/dev/null
unfunction dump_src_trace src_a src_b 2>/dev/null
unfunction recursive_dump 2>/dev/null
unfunction report_error api_call db_query auth_check 2>/dev/null
unfunction my_introspect_fn 2>/dev/null
echo "  all done."

# === ztest assertions ===
# Top-level: funcstack empty
zassert_eq "${#funcstack}" "0" "funcstack empty at top level"
# Inside nested calls: funcstack length grows
my_a() {
    typeset -ga MY_STACK
    MY_STACK=("${funcstack[@]}")
}
my_b() { my_a; }
my_c() { my_b; }
my_c
zassert_eq "${#MY_STACK}" "3" "funcstack depth 3 inside nested calls"
zassert_eq "${MY_STACK[1]}" "my_a" "innermost frame is my_a"
zassert_eq "${MY_STACK[2]}" "my_b" "my_b in stack"
zassert_eq "${MY_STACK[3]}" "my_c" "outermost frame is my_c"
# functions assoc has the body
my_simple_fn() { echo "marker_string_xyz"; }
zassert_contains "${functions[my_simple_fn]}" "marker_string_xyz" "functions assoc has body"
# ZSH_ARGZERO / $0 set
zassert_ok "$0" "\$0 set"
ztest_run
