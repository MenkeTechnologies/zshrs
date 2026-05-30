#!/usr/bin/env zshrs
# Ring buffer (circular array) — fixed-size FIFO w/ overwrite on full.
# Uses global RB_RESULT instead of cmd-sub (subshells don't propagate state in zshrs).

CAPACITY=8
typeset -a RING
typeset -gi HEAD=1 TAIL=1 COUNT=0
RB_RESULT=""

rb_init() {
    RING=()
    local i
    for ((i=1; i<=CAPACITY; i++)); do RING[i]=""; done
    HEAD=1
    TAIL=1
    COUNT=0
}

rb_push() {
    local v=$1
    RING[$TAIL]=$v
    TAIL=$(( TAIL % CAPACITY + 1 ))
    if (( COUNT == CAPACITY )); then
        HEAD=$(( HEAD % CAPACITY + 1 ))
    else
        (( COUNT++ ))
    fi
}

rb_pop() {
    if (( COUNT == 0 )); then
        RB_RESULT=""
        return 1
    fi
    RB_RESULT="${RING[$HEAD]}"
    HEAD=$(( HEAD % CAPACITY + 1 ))
    (( COUNT-- ))
    return 0
}

rb_print() {
    printf "  state: ["
    local i pos
    pos=$HEAD
    for ((i=0; i<COUNT; i++)); do
        if (( i > 0 )); then printf " "; fi
        printf "%s" "${RING[$pos]}"
        pos=$(( pos % CAPACITY + 1 ))
    done
    printf "] (head=%d tail=%d count=%d)\n" $HEAD $TAIL $COUNT
}

rb_print_raw() {
    printf "  raw:   ["
    local i
    for ((i=1; i<=CAPACITY; i++)); do
        if (( i > 1 )); then printf " "; fi
        local v="${RING[$i]}"
        [[ -z $v ]] && v="_"
        printf "%s" "$v"
    done
    printf "]\n"
}

echo "── push 5 items ──"
rb_init
for v in A B C D E; do
    rb_push $v
done
rb_print
rb_print_raw

echo
echo "── fill to capacity (push 3 more) ──"
for v in F G H; do
    rb_push $v
done
rb_print
rb_print_raw
echo "  is_full: $([[ $COUNT == $CAPACITY ]] && echo yes || echo no)"

echo
echo "── overwrite: push 4 more (X Y Z W) ──"
for v in X Y Z W; do
    rb_push $v
done
echo "  oldest 4 (A B C D) should be gone"
rb_print
rb_print_raw

echo
echo "── pop 3 ──"
for i in 1 2 3; do
    rb_pop
    echo "  pop: $RB_RESULT"
done
rb_print

echo
echo "── peek ──"
echo "  peek (HEAD): ${RING[$HEAD]}"
rb_print

echo
echo "── pop until empty ──"
while (( COUNT > 0 )); do
    rb_pop
    echo "  pop: $RB_RESULT"
done
echo "  is_empty: $([[ $COUNT == 0 ]] && echo yes || echo no)"

echo
echo "── log-buffer use case (latest N entries) ──"
rb_init
log_msgs=(
    "boot"
    "load config"
    "open db"
    "accept conn 1"
    "accept conn 2"
    "accept conn 3"
    "request /api"
    "request /admin"
    "auth failure"
    "request /login"
    "session start"
    "request /home"
    "logout"
    "shutdown"
)
for msg in "${log_msgs[@]}"; do
    rb_push "$msg"
done
echo "  ingested ${#log_msgs} messages, kept latest $CAPACITY:"
while (( COUNT > 0 )); do
    rb_pop
    echo "    $RB_RESULT"
done

echo
echo "── frame buffer style (overwrite tick counter) ──"
rb_init
CAPACITY=6
RING=("" "" "" "" "" "")
HEAD=1; TAIL=1; COUNT=0
for tick in {1..15}; do
    rb_push "f$tick"
done
echo "  after 15 frames pushed into capacity-6 buffer:"
rb_print

echo
echo "── stats ──"
echo "  capacity: $CAPACITY"
echo "  current count: $COUNT"
echo "  ops: rb_push / rb_pop / rb_print"
