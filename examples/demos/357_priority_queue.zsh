#!/usr/bin/env zshrs
# Priority queue — min-heap + Dijkstra application.

typeset -a HEAP

heap_init() { HEAP=(); }

heap_size() { echo ${#HEAP}; }

heap_push() {
    local v=$1
    HEAP+=($v)
    local i=${#HEAP}
    while (( i > 1 )); do
        local parent=$(( i / 2 ))
        if (( HEAP[parent] > HEAP[i] )); then
            local tmp=${HEAP[parent]}
            HEAP[parent]=${HEAP[i]}
            HEAP[i]=$tmp
            i=$parent
        else
            break
        fi
    done
}

heap_pop() {
    if (( ${#HEAP} == 0 )); then
        HEAP_RESULT=""
        return 1
    fi
    HEAP_RESULT=${HEAP[1]}
    local n=${#HEAP}
    HEAP[1]=${HEAP[n]}
    HEAP[$n]=()
    local i=1
    local size=${#HEAP}
    while true; do
        local l=$(( 2 * i ))
        local r=$(( 2 * i + 1 ))
        local smallest=$i
        if (( l <= size && HEAP[l] < HEAP[smallest] )); then
            smallest=$l
        fi
        if (( r <= size && HEAP[r] < HEAP[smallest] )); then
            smallest=$r
        fi
        if (( smallest != i )); then
            local tmp=${HEAP[i]}
            HEAP[i]=${HEAP[smallest]}
            HEAP[smallest]=$tmp
            i=$smallest
        else
            break
        fi
    done
    return 0
}

heap_peek() {
    if (( ${#HEAP} == 0 )); then
        echo ""
        return 1
    fi
    echo ${HEAP[1]}
}

# String-prefix heap (for "priority:value" entries).
# Push as "PPP value"; sorts on first field.
typeset -a PRIO_HEAP

ph_init() { PRIO_HEAP=(); }

ph_push() {
    local prio=$1 val=$2
    PRIO_HEAP+=("$prio $val")
    local i=${#PRIO_HEAP}
    while (( i > 1 )); do
        local parent=$(( i / 2 ))
        local p_prio="${PRIO_HEAP[parent]%% *}"
        local i_prio="${PRIO_HEAP[i]%% *}"
        if (( p_prio > i_prio )); then
            local tmp="${PRIO_HEAP[parent]}"
            PRIO_HEAP[parent]="${PRIO_HEAP[i]}"
            PRIO_HEAP[i]="$tmp"
            i=$parent
        else
            break
        fi
    done
}

ph_pop() {
    if (( ${#PRIO_HEAP} == 0 )); then
        PH_RESULT=""
        return 1
    fi
    PH_RESULT="${PRIO_HEAP[1]}"
    local n=${#PRIO_HEAP}
    PRIO_HEAP[1]="${PRIO_HEAP[n]}"
    PRIO_HEAP[$n]=()
    local i=1
    local size=${#PRIO_HEAP}
    while true; do
        local l=$(( 2 * i ))
        local r=$(( 2 * i + 1 ))
        local smallest=$i
        if (( l <= size )); then
            local l_prio="${PRIO_HEAP[l]%% *}"
            local s_prio="${PRIO_HEAP[smallest]%% *}"
            if (( l_prio < s_prio )); then smallest=$l; fi
        fi
        if (( r <= size )); then
            local r_prio="${PRIO_HEAP[r]%% *}"
            local s_prio="${PRIO_HEAP[smallest]%% *}"
            if (( r_prio < s_prio )); then smallest=$r; fi
        fi
        if (( smallest != i )); then
            local tmp="${PRIO_HEAP[i]}"
            PRIO_HEAP[i]="${PRIO_HEAP[smallest]}"
            PRIO_HEAP[smallest]="$tmp"
            i=$smallest
        else
            break
        fi
    done
    return 0
}

echo "── basic min-heap ──"
heap_init
for v in 5 2 8 1 9 3 7 4 6; do
    heap_push $v
done
echo "  pushed: 5 2 8 1 9 3 7 4 6"
echo "  size: $(heap_size)"
echo "  peek: $(heap_peek)"

echo
echo "  pop all (heap sort):"
out=""
size=$(heap_size)
for ((i=1; i<=size; i++)); do
    heap_pop
    out+="$HEAP_RESULT "
done
echo "  $out"

echo
echo "── stream max-N via min-heap ──"
# Keep heap of size K; pop smallest when over.
TOP_K=5
heap_init
input=(50 23 78 12 99 45 67 89 34 56 11 88 22 90 33)
for v in "${input[@]}"; do
    if (( ${#HEAP} < TOP_K )); then
        heap_push $v
    elif (( v > HEAP[1] )); then
        heap_pop > /dev/null
        heap_push $v
    fi
done
echo "  stream: ${input[@]}"
echo "  top 5: $(heap_peek)"
echo "  heap contents (not sorted): ${HEAP[*]}"
echo "  sorted:"
out=""
size=$(heap_size)
for ((i=1; i<=size; i++)); do
    heap_pop
    out+="$HEAP_RESULT "
done
echo "  $out"
echo "  expected: 78 88 89 90 99"

echo
echo "── priority queue (string + priority) ──"
ph_init
ph_push 5 "low priority task"
ph_push 1 "urgent emergency"
ph_push 3 "normal task"
ph_push 1 "another urgent"
ph_push 8 "future maintenance"
ph_push 2 "important"

echo "  queue contents (heap order, not sorted):"
for entry in "${PRIO_HEAP[@]}"; do
    echo "    [$entry]"
done

echo
echo "  pop in priority order:"
n=${#PRIO_HEAP}
for ((i=1; i<=n; i++)); do
    ph_pop
    echo "    $PH_RESULT"
done

echo
echo "── Dijkstra via priority queue ──"
typeset -A NBRS
NBRS=(
    A "B:7 C:9 F:14"
    B "A:7 C:10 D:15"
    C "A:9 B:10 D:11 F:2"
    D "B:15 C:11 E:6"
    E "D:6 F:9"
    F "A:14 C:2 E:9"
)

dijkstra() {
    local source=$1
    typeset -A dist
    typeset -a nodes
    nodes=(A B C D E F)
    for n in "${nodes[@]}"; do dist[$n]=999999; done
    dist[$source]=0

    ph_init
    ph_push 0 $source

    while (( ${#PRIO_HEAP} > 0 )); do
        ph_pop
        local entry="$PH_RESULT"
        local d="${entry%% *}"
        local cur="${entry##* }"
        if (( d > dist[$cur] )); then continue; fi
        for nbr_w in ${=NBRS[$cur]}; do
            local nbr="${nbr_w%:*}"
            local w="${nbr_w#*:}"
            local new_d=$(( d + w ))
            if (( new_d < dist[$nbr] )); then
                dist[$nbr]=$new_d
                ph_push $new_d $nbr
            fi
        done
    done

    echo "  distances from $source:"
    for n in "${nodes[@]}"; do
        printf "    %s: %d\n" "$n" "${dist[$n]}"
    done
}

dijkstra A

echo
echo "── complexity ──"
echo "  push/pop:  O(log n)"
echo "  build:     O(n)"
echo "  peek:      O(1)"
echo "  Dijkstra:  O((V+E) log V) with binary heap"
echo "  Dijkstra:  O(V log V + E) with Fibonacci heap"
echo
echo "  applications:"
echo "    task scheduling, event-driven sim"
echo "    Dijkstra shortest path"
echo "    Huffman coding"
echo "    A* search"
echo "    k-way merge"
echo "    top-k from a stream"

# === ztest assertions ===
heap_init
zassert_eq "$(heap_size)" "0" "empty heap size"
heap_push 5; heap_push 2; heap_push 8; heap_push 1; heap_push 9
zassert_eq "$(heap_size)" "5" "size after 5 pushes"
zassert_eq "$(heap_peek)" "1" "min after 5 pushes"
heap_pop
zassert_eq "$HEAP_RESULT" "1" "pop returns min"
heap_pop
zassert_eq "$HEAP_RESULT" "2" "second pop returns next-min"
# Build a full heap-sort sequence.
heap_init
for v in 7 3 5 1 4 2 8 6; do heap_push $v; done
out=""
size=$(heap_size)
for ((i=1; i<=size; i++)); do
    heap_pop
    out+="$HEAP_RESULT "
done
zassert_eq "${out% }" "1 2 3 4 5 6 7 8" "heap sort"
# Priority queue
ph_init
ph_push 5 "low"
ph_push 1 "urgent"
ph_push 3 "normal"
ph_pop
zassert_eq "$PH_RESULT" "1 urgent" "highest priority (lowest num) popped first"
# Dijkstra distances
# (dist is typeset -A inside dijkstra; we test the printed distances)
dijkstra A > /tmp/_pq357_dij
zassert_contains "$(cat /tmp/_pq357_dij)" "B: 7"  "Dijkstra: A→B = 7"
zassert_contains "$(cat /tmp/_pq357_dij)" "C: 9"  "Dijkstra: A→C = 9"
zassert_contains "$(cat /tmp/_pq357_dij)" "F: 11" "Dijkstra: A→F = 11"
rm -f /tmp/_pq357_dij
ztest_run
