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
        stack=("${stack[@]:0:$((n-1))}")
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
zassert_eq "$(size)" "0"       "stack drained by the pop loop above"
push one; push two; push three
zassert_eq "$(size)" "3"       "three pushes"
peek
zassert_eq "$STACK_RESULT" "three" "peek sees the top without popping"
zassert_eq "$(size)" "3"       "peek does not shrink the stack"
pop
zassert_eq "$STACK_RESULT" "three" "pop returns the top"
zassert_eq "$(size)" "2"       "pop shrinks the stack"
pop; pop
zassert_eq "$STACK_RESULT" "one" "last pop returns the bottom"
zassert_eq "$(size)" "0"       "stack empty again"
ztest_run
