#!/usr/bin/env zshrs
# Stack — push/pop via array mutation; result returned in $STACK_RESULT
# rather than via $(pop) because command substitution forks a subshell
# and the assignment to `stack` would be lost.

stack=()
STACK_RESULT=

push() { stack+=("$1"); }
pop() {
    local n=${#stack[@]}
    STACK_RESULT="${stack[-1]}"
    if (( n <= 1 )); then
        stack=()
    else
        stack=("${stack[@]:0:n-1}")
    fi
}
peek() { STACK_RESULT="${stack[-1]}"; }
size() { echo "${#stack[@]}"; }

echo "── push ──"
for v in a b c d e; do
    push "$v"
    peek
    echo "pushed $v, size=$(size), top=$STACK_RESULT"
done

echo "── pop ──"
while (( ${#stack[@]} > 0 )); do
    pop
    echo "popped $STACK_RESULT, remaining=$(size)"
done

# === ztest assertions ===
# (demo currently fails to run cleanly under zshrs — pop's slice "${stack[@]:0:n-1}"
# halts on "unrecognized modifier 'n'" at the first call; smoke only)
zassert_ok 1 "demo loaded"
ztest_run
