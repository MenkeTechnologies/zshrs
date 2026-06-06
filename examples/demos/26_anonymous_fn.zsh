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

# === ztest assertions ===
zassert_eq "$(() { echo "anon $1 $2"; } a b)" "anon a b"  "two-arg anon"
zassert_eq "$result" "81"                                "9 squared via anon"
zassert_eq "${secret:-unset}" "unset"                    "local in anon does not leak"
sum=$(() { local t=0; for n in "$@"; do (( t+=n )); done; echo $t; } 1 2 3 4 5)
zassert_eq "$sum" "15"                                   "anon-fn varargs sum"
out=$(() { local i; for i in a b c; do echo "$i"; done; })
zassert_contains "$out" "a"  "anon loops produce a"
zassert_contains "$out" "b"  "anon loops produce b"
zassert_contains "$out" "c"  "anon loops produce c"
# Anonymous fn return code propagates.
() { return 0; } && zassert_ok 1 "anon returning 0 → success branch" || zassert_ok 0 "anon returning 0 → success"
() { return 1; } || zassert_ok 1 "anon returning 1 → failure branch" && true
ztest_run
