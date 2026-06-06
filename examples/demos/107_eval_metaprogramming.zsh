#!/usr/bin/env zshrs
# Eval-based metaprogramming patterns.
# Ported from Src/builtin.c bin_eval + Src/exec.c parse_string.

echo "── compose a fn from a template ──"
build_getter() {
    local name=$1 val=$2
    eval "
        ${name}_get() {
            echo $val
        }
    "
}
build_getter pi 3.14159
build_getter e 2.71828
pi_get
e_get

echo "── parameterized class-like factory ──"
define_record() {
    local cls=$1
    shift
    local fields=("$@")
    # Constructor
    eval "
        ${cls}_new() {
            local id=\$1
            shift
            for f in ${fields[@]}; do
                eval \"${cls}_\${id}_\${f}=\\\"\\\$1\\\"\"
                shift
            done
        }
        ${cls}_show() {
            local id=\$1
            echo '── $cls #'\$id ' ──'
            for f in ${fields[@]}; do
                local var=${cls}_\${id}_\${f}
                echo \"  \$f = \${(P)var}\"
            done
        }
    "
}

define_record person name age role
person_new 1 alice 30 admin
person_new 2 bob 25 user
person_show 1
person_show 2

echo "── runtime dispatch via name ──"
for f in person_new person_show pi_get; do
    if (( $+functions[$f] )); then
        echo "  $f defined"
    fi
done

echo "── code generation via printf+eval ──"
fn_body=$(printf 'echo "name=%s value=%d"' "computed" 42)
eval "computed_fn() { $fn_body; }"
computed_fn

echo "── conditional definition ──"
DEBUG=1
if (( DEBUG )); then
    eval 'log() { echo "[DEBUG] $*" >&2; }'
else
    eval 'log() { :; }'
fi
log "this is a debug message"

echo "── reflective inspection via \$functions ──"
echo "computed_fn body:"
echo "${functions[computed_fn]}"

# === ztest assertions ===
# build_getter dynamically created pi_get and e_get.
zassert_eq "$(pi_get)"   "3.14159"   "pi_get via eval"
zassert_eq "$(e_get)"    "2.71828"   "e_get via eval"
# computed_fn echoes templated message.
zassert_eq "$(computed_fn)"  "name=computed value=42"  "computed_fn body"
# log when DEBUG=1 writes to stderr only.
out=$(log "hello" 2>&1)
zassert_eq "$out" "[DEBUG] hello"  "log writes [DEBUG] prefix"
# Dynamic eval'd fn registered in functions hash.
zassert_ok "$+functions[pi_get]"      "pi_get in functions hash"
zassert_ok "$+functions[computed_fn]" "computed_fn in functions hash"
# Build another at runtime + invoke.
build_getter c 299792458
zassert_eq "$(c_get)"   "299792458"   "build_getter c"
ztest_run
