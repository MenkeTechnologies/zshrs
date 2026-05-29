#!/usr/bin/env zshrs
# Variable indirection — ${(P)var}, eval, name-resolution by string.
# Ported from Src/subst.c paramsubst (P) flag + Src/builtin.c bin_eval.

echo "── (P) name-reference indirection ──"
real_name="origin_value"
proxy=real_name
echo "direct:   $real_name"
echo "via (P):  ${(P)proxy}"

echo "── lookup by computed name ──"
declare item_apple=10
declare item_banana=20
declare item_cherry=30

for fruit in apple banana cherry; do
    name="item_$fruit"
    echo "$fruit: ${(P)name}"
done

echo "── inverse: variable-as-name + assign via eval ──"
target=foo
eval "$target=42"
echo "after eval-assign: foo=$foo"

target=bar
eval "$target=hello-world"
echo "bar=$bar"

echo "── (P) on arrays ──"
arr=(alpha beta gamma)
ref=arr
echo "(P)ref: ${(P)ref}"

echo "── compose dispatchers via name lookup ──"
dispatch() {
    local name=$1
    shift
    if (( $+functions[$name] )); then
        $name "$@"
    else
        echo "no fn '$name'"
    fi
}
foo_handler() { echo "foo called: $@"; }
bar_handler() { echo "bar called: $@"; }
dispatch foo_handler a b c
dispatch bar_handler one two
dispatch nonexistent

echo "── \$+var existence test ──"
existing=present
unset notdefined
echo "existing: $+existing"
echo "notdefined: $+notdefined"

echo "── ${(t)var} type info ──"
typeset -i n=42
typeset -a arr2=(a b c)
typeset -A m
m[k]=v
echo "n type: ${(t)n}"
echo "arr2 type: ${(t)arr2}"
echo "m type: ${(t)m}"

echo "── auto-pair: build setter+getter from prefix ──"
make_accessor() {
    local prefix=$1
    eval "${prefix}_set() { ${prefix}_value=\"\$1\"; }"
    eval "${prefix}_get() { echo \"\$${prefix}_value\"; }"
}
make_accessor counter
counter_set 100
counter_get
counter_set 200
counter_get
