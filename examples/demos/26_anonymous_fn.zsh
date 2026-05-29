#!/usr/bin/env zshrs
# Anonymous functions — () { ... } called inline.

echo "── basic ──"
() { echo "anon: $1 $2"; } hello world

echo "── used to scope locals ──"
() {
    local secret=12345
    echo "inside: secret=$secret"
}

# `secret` is gone outside the anonymous fn.
echo "outside: secret=${secret:-unset}"

echo "── as a function constructor pattern ──"
() {
    local i
    for i in alpha beta gamma; do
        printf "  iter[%s]\n" $i
    done
}

echo "── with arg processing ──"
() {
    local total=0
    for n in "$@"; do (( total += n )); done
    echo "sum-from-anon: $total"
} 1 2 3 4 5

echo "── returning via stdout ──"
result=$(() { echo $(( $1 * $1 )) } 9)
echo "9 squared (anon): $result"
