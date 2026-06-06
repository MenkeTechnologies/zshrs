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

# === ztest assertions ===
zassert_eq "${#CACHE[@]}" "0" "reset clears CACHE"
zassert_eq "${#LRU_ORDER[@]}" "0" "reset clears LRU_ORDER"
# rebuild and replay the eviction path under assertions
cache_set k1 v1 >/dev/null
cache_set k2 v2 >/dev/null
cache_set k3 v3 >/dev/null
zassert_eq "${#CACHE[@]}" "3" "cache at capacity"
zassert_eq "${LRU_ORDER[1]}" "k1" "oldest = k1"
zassert_eq "${LRU_ORDER[3]}" "k3" "newest = k3"
cache_get k1 >/dev/null
zassert_eq "${LRU_ORDER[3]}" "k1" "k1 moves to MRU"
zassert_eq "${LRU_ORDER[1]}" "k2" "k2 now oldest"
cache_set k4 v4 >/dev/null
zassert_eq "${CACHE[k2]+x}" "" "k2 evicted as oldest"
zassert_eq "${#CACHE[@]}"   "3" "size held at capacity"
zassert_contains "$(cache_get missing)" "miss" "miss reported for unknown"
ztest_run
