#!/usr/bin/env zshrs
# Parameter quoting flags — (q), (qq), (qqq), (qqqq).
# Ported from Src/subst.c paramsubst quote dispatch.

s='hello world "with quotes" & $vars'

echo "── original ──"
echo "$s"
echo "length: ${#s}"

echo "── (q) — backslash-quote shell metas ──"
echo "${(q)s}"

echo "── (qq) — single-quote ──"
echo "${(qq)s}"

echo "── (qqq) — double-quote ──"
echo "${(qqq)s}"

echo "── (qqqq) — \$'...' escaped ──"
echo "${(qqqq)s}"

echo "── compare side by side ──"
texts=(
    "simple"
    "with space"
    "with 'single'"
    'with "double"'
    'has $dollar'
    'has\\backslash'
    'tab	there'
)
for t in "${texts[@]}"; do
    printf "%-30s q=[%s]\n" "'$t'" "${(q)t}"
done

echo "── round-trip via eval ──"
for original in "hello world" "it's fine" "with \"quote\""; do
    encoded="${(q)original}"
    decoded=$(eval "echo $encoded")
    if [[ "$original" == "$decoded" ]]; then
        echo "OK: $original → $encoded → $decoded"
    else
        echo "FAIL: $original → $encoded → $decoded"
    fi
done

echo "── build a safe command ──"
build_cmd() {
    local cmd=$1
    shift
    local out="$cmd"
    for arg in "$@"; do
        out+=" ${(q)arg}"
    done
    echo "$out"
}
build_cmd echo "first arg" "second 'arg" 'third "arg' '$shellvar' '`cmdsubst`'
