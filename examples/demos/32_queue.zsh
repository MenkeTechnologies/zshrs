#!/usr/bin/env zshrs
# Queue — FIFO via array mutation; result returned in $QUEUE_RESULT
# (see 31_stack.zsh for the reason $(dequeue) wouldn't mutate).

queue=()
QUEUE_RESULT=

enqueue() { queue+=("$1"); }
dequeue() {
    local n=${#queue[@]}
    QUEUE_RESULT="${queue[1]}"
    if (( n <= 1 )); then
        queue=()
    else
        queue=("${queue[@]:1}")
    fi
}
size() { echo "${#queue[@]}"; }

echo "── enqueue ──"
for v in apple banana cherry date elderberry; do
    enqueue "$v"
    echo "enqueued $v, size=$(size)"
done

echo "── dequeue (FIFO) ──"
while (( ${#queue[@]} > 0 )); do
    dequeue
    echo "dequeued $QUEUE_RESULT, remaining=$(size)"
done
