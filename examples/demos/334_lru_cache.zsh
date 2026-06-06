#!/usr/bin/env zshrs
# LRU cache — O(1) get/put via doubly-linked list + hash.

CAPACITY=5
typeset -A KEY_TO_VAL KEY_NEXT KEY_PREV
HEAD="__head"
TAIL="__tail"

lru_init() {
    KEY_TO_VAL=()
    KEY_NEXT=()
    KEY_PREV=()
    KEY_NEXT[$HEAD]=$TAIL
    KEY_PREV[$TAIL]=$HEAD
    KEY_PREV[$HEAD]=""
    KEY_NEXT[$TAIL]=""
}

# Unlink a node.
unlink() {
    local k=$1
    local p=${KEY_PREV[$k]}
    local n=${KEY_NEXT[$k]}
    KEY_NEXT[$p]=$n
    KEY_PREV[$n]=$p
    unset "KEY_PREV[$k]"
    unset "KEY_NEXT[$k]"
}

# Insert k right after HEAD (most recently used).
insert_at_head() {
    local k=$1
    local first=${KEY_NEXT[$HEAD]}
    KEY_NEXT[$HEAD]=$k
    KEY_PREV[$k]=$HEAD
    KEY_NEXT[$k]=$first
    KEY_PREV[$first]=$k
}

lru_get() {
    local k=$1
    if [[ -z ${KEY_TO_VAL[$k]+x} ]]; then
        echo "MISS"
        return 1
    fi
    # Move to head.
    unlink $k
    insert_at_head $k
    echo "${KEY_TO_VAL[$k]}"
    return 0
}

lru_put() {
    local k=$1 v=$2
    if [[ -n ${KEY_TO_VAL[$k]+x} ]]; then
        # Update + move to head.
        KEY_TO_VAL[$k]=$v
        unlink $k
        insert_at_head $k
        return
    fi
    if (( ${#KEY_TO_VAL} >= CAPACITY )); then
        # Evict least recently used (just before TAIL).
        local lru=${KEY_PREV[$TAIL]}
        unlink $lru
        unset "KEY_TO_VAL[$lru]"
        echo "  evicted: $lru"
    fi
    KEY_TO_VAL[$k]=$v
    insert_at_head $k
}

lru_size() { echo ${#KEY_TO_VAL}; }

lru_print() {
    printf "  ["
    local cur=${KEY_NEXT[$HEAD]} first=1
    while [[ $cur != $TAIL && -n $cur ]]; do
        if (( ! first )); then printf " → "; fi
        printf "%s:%s" "$cur" "${KEY_TO_VAL[$cur]}"
        first=0
        cur=${KEY_NEXT[$cur]}
    done
    printf "] (size=%d)\n" $(lru_size)
}

echo "── basic LRU operations (capacity=$CAPACITY) ──"
lru_init
echo "  put A=1"; lru_put A 1; lru_print
echo "  put B=2"; lru_put B 2; lru_print
echo "  put C=3"; lru_put C 3; lru_print
echo "  put D=4"; lru_put D 4; lru_print
echo "  put E=5"; lru_put E 5; lru_print

echo
echo "── eviction (cache full) ──"
echo "  put F=6 (should evict A)"; lru_put F 6; lru_print

echo
echo "── access updates MRU ──"
echo "  get C: $(lru_get C)"
lru_print

echo "  get B: $(lru_get B)"
lru_print

echo
echo "── miss ──"
echo "  get A: $(lru_get A) (already evicted)"

echo
echo "── update existing ──"
echo "  put E=50 (update, move to head)"
lru_put E 50
lru_print

echo
echo "── stress test: 20 ops ──"
lru_init
CAPACITY=3
hit_count=0
miss_count=0
for op in "put X 1" "put Y 2" "put Z 3" "get X" "put W 4" "get Z" "put V 5" "get Y" "put X 10" "get V"; do
    set -- ${=op}
    if [[ $1 == put ]]; then
        echo "  $op → ..."
        lru_put $2 $3
        lru_print
    else
        v=$(lru_get $2)
        if [[ $v == MISS ]]; then
            (( miss_count++ ))
        else
            (( hit_count++ ))
        fi
        echo "  $op → $v"
        lru_print
    fi
done

echo
echo "  hits: $hit_count, misses: $miss_count"
echo "  hit rate: $(( hit_count * 100 / (hit_count + miss_count) ))%"

echo
echo "── working-set simulation ──"
CAPACITY=4
lru_init
RANDOM=42
accesses=""
hits=0
misses=0
for ((i=0; i<30; i++)); do
    # 80% access from working set {a,b,c}, 20% random.
    if (( RANDOM % 5 < 4 )); then
        keys=(a b c)
    else
        keys=(d e f g h i j)
    fi
    k=${keys[$(( RANDOM % ${#keys} + 1 ))]}
    accesses+="$k "
    v=$(lru_get $k 2>/dev/null)
    if [[ $v == MISS ]]; then
        (( misses++ ))
        lru_put $k $RANDOM > /dev/null 2>&1
    else
        (( hits++ ))
    fi
done
echo "  accesses: $accesses"
echo "  hits=$hits  misses=$misses  hit rate=$(( hits * 100 / (hits + misses) ))%"
echo "  (working set ⊂ capacity → high hit rate expected)"
lru_print

echo
echo "── stats ──"
echo "  ops: O(1) get, O(1) put, O(1) eviction"
echo "  storage: doubly linked list + hash map"
echo "  apps: page cache, CPU cache, web cache, CDN"

# === ztest assertions ===
CAPACITY=3
lru_init
lru_put A 1 > /dev/null
lru_put B 2 > /dev/null
lru_put C 3 > /dev/null
zassert_eq "$(lru_size)" 3 "size after 3 puts"
zassert_eq "$(lru_get A)" "1" "get A"
zassert_eq "$(lru_get B)" "2" "get B"
zassert_eq "$(lru_get C)" "3" "get C"
zassert_eq "$(lru_get Z)" "MISS" "miss"
lru_put D 4 > /dev/null    # evicts A (LRU)
zassert_eq "$(lru_get A)" "MISS" "A evicted"
zassert_eq "$(lru_get B)" "2"    "B retained"
zassert_eq "$(lru_size)"  3      "size still 3"
lru_put B 22 > /dev/null   # update existing
zassert_eq "$(lru_get B)" "22"   "B updated"
ztest_run
