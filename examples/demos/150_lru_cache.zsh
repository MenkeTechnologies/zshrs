#!/usr/bin/env zshrs
# LRU cache — fixed-capacity key/value store with eviction.

typeset -A CACHE
typeset -a LRU_ORDER   # most-recently-used at end
typeset -i CAPACITY=3

cache_get() {
    local key=$1
    if [[ -z ${CACHE[$key]+x} ]]; then
        echo "miss: $key"
        return 1
    fi
    # Move to end of LRU.
    local new_order=()
    for k in "${LRU_ORDER[@]}"; do
        [[ $k != $key ]] && new_order+=("$k")
    done
    new_order+=("$key")
    LRU_ORDER=("${new_order[@]}")
    echo "hit: $key → ${CACHE[$key]}"
}

cache_set() {
    local key=$1 val=$2
    if [[ -n ${CACHE[$key]+x} ]]; then
        # Update existing; move to end.
        CACHE[$key]=$val
        local new_order=()
        for k in "${LRU_ORDER[@]}"; do
            [[ $k != $key ]] && new_order+=("$k")
        done
        new_order+=("$key")
        LRU_ORDER=("${new_order[@]}")
        echo "updated: $key → $val"
        return
    fi
    if (( ${#CACHE[@]} >= CAPACITY )); then
        # Evict LRU (front of order).
        local victim=${LRU_ORDER[1]}
        unset "CACHE[$victim]"
        LRU_ORDER=("${LRU_ORDER[@]:1}")
        echo "evicted: $victim"
    fi
    CACHE[$key]=$val
    LRU_ORDER+=("$key")
    echo "set: $key → $val"
}

cache_dump() {
    echo "── cache state (cap=$CAPACITY, size=${#CACHE[@]}) ──"
    echo "LRU order (oldest→newest): ${LRU_ORDER[@]}"
    for k in "${LRU_ORDER[@]}"; do
        echo "  $k = ${CACHE[$k]}"
    done
}

echo "── fill cache ──"
cache_set a 1
cache_set b 2
cache_set c 3
cache_dump

echo "── access a (becomes MRU) ──"
cache_get a
cache_dump

echo "── add d (evicts b — oldest) ──"
cache_set d 4
cache_dump

echo "── update c (no eviction) ──"
cache_set c 30
cache_dump

echo "── miss ──"
cache_get nonexistent

echo "── access pattern with hot key ──"
for i in 1 2 3 4 5; do
    cache_get c >/dev/null
done
cache_set e 5  # should evict a (c stayed hot)
cache_dump

echo "── reset by clearing ──"
CACHE=()
LRU_ORDER=()
echo "size: ${#CACHE[@]}"
