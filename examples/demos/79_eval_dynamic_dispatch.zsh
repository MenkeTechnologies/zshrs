#!/usr/bin/env zshrs
# eval + dynamic dispatch — late-bound command resolution.
# Ported from zsh's Src/builtin.c bin_eval + Src/exec.c iscom path.

echo "── eval basic ──"
expr='echo "computed at runtime: $((2*21))"'
eval $expr

echo "── eval with deferred expansion ──"
var=42
fmt='echo "var was \$var; got \$var=$var"'
eval $fmt

echo "── eval into a var ──"
result=$(eval 'echo $(( 2 ** 16 ))')
echo "2^16 via eval: $result"

echo "── dynamic command name ──"
cmd=echo
$cmd "dynamically dispatched"
cmd=printf
$cmd "%s\n" "via printf"

echo "── dispatch table via assoc ──"
typeset -A handlers
handlers[greet]='echo "hello world"'
handlers[count]='echo "count: $#args"'
handlers[upper]='echo "${args[1]:u}"'

for op in greet count upper; do
    args=(one two three)
    eval "${handlers[$op]}"
done

echo "── poor-mans macro: define fn from string ──"
def_fn() {
    eval "$1() { echo \"defined $1: \$@\"; }"
}
def_fn alpha
def_fn beta
def_fn gamma
alpha A1 A2
beta B1
gamma G1 G2 G3

echo "── eval arithmetic expression ──"
ops=("2 + 3" "10 * 5" "100 / 7" "2 ** 8")
for op in "${ops[@]}"; do
    val=$(eval "echo \$(( $op ))")
    echo "$op = $val"
done

echo "── recursive dispatch ──"
typeset -A actions
actions[hello]='greet'
actions[exit]='quit'
greet() { echo "hi there"; }
quit() { echo "bye"; }
for input in hello exit; do
    cmd=${actions[$input]}
    $cmd
done

# === ztest assertions ===
expr='echo "computed at runtime: $((2*21))"'
zassert_eq "$(eval $expr)"                "computed at runtime: 42"   "basic eval"
zassert_eq "$(eval 'echo $(( 2 ** 16 ))')" "65536"                    "eval into capture"
def_fn delta
zassert_eq "$(delta D1 D2)"               "defined delta: D1 D2"      "eval-defined fn"
def_fn echoer
zassert_eq "$(echoer single)"             "defined echoer: single"    "second eval-defined fn"
ops=("2 + 3" "10 * 5" "100 / 7" "2 ** 8")
zassert_eq "$(eval "echo \$(( ${ops[1]} ))")" "5"   "eval ops[1]"
zassert_eq "$(eval "echo \$(( ${ops[3]} ))")" "14"  "eval ops[3] (100/7)"
typeset -A H
H[k]='echo "hit-k"'
zassert_eq "$(eval ${H[k]})"              "hit-k"                     "dispatch table assoc lookup"
ztest_run

