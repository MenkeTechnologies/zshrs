#!/usr/bin/env zshrs
# Function introspection — typeset -f, which, $functions[].
# Ported from Src/builtin.c bin_functions + Src/Modules/parameter.c
# (functions hash exposure).

# Define some functions.
hello() { echo "hi from hello"; }
greet() { hello; echo "extras for $1"; }

echo "── typeset -f (list def) ──"
typeset -f hello
echo
typeset -f greet

echo "── which (alias-aware) ──"
which hello
which greet

echo "── \$functions[] (assoc of all fn bodies) ──"
echo "function count: ${#functions[@]} (includes built-ins / inherited)"
echo "hello's body (first line):"
echo "${functions[hello]}" | head -1

echo "── add a function dynamically via eval ──"
eval 'dynamic_fn() { echo "I was defined at runtime: $*"; }'
typeset -f dynamic_fn
dynamic_fn alpha beta

echo "── modify a function body ──"
functions[hello]='echo "redefined hello"'
hello

echo "── delete a function ──"
unfunction hello
if (( $+functions[hello] )); then
    echo "hello still defined"
else
    echo "hello gone"
fi

echo "── functions matching a pattern ──"
foo_one() { :; }
foo_two() { :; }
bar_baz() { :; }
echo "matching foo_*:"
for fn in ${(k)functions}; do
    [[ $fn == foo_* ]] && echo "  $fn"
done
unfunction foo_one foo_two bar_baz 2>/dev/null

echo "── trace a function with set -x via local ──"
traced_fn() {
    setopt local_options
    setopt xtrace
    echo "in body"
}
traced_fn 2>&1 | head -3 || true
