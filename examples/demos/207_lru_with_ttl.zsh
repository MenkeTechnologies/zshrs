#!/usr/bin/env zshrs
# Time-to-live cache — expires entries after N seconds.

zmodload zsh/datetime 2>/dev/null

typeset -A CACHE_VAL
typeset -A CACHE_EXP

cache_set() {
    local key=$1 val=$2 ttl=$3
    CACHE_VAL[$key]=$val
    CACHE_EXP[$key]=$(( EPOCHSECONDS + ttl ))
    echo "  set $key=$val (expires +${ttl}s)"
}

cache_get() {
    local key=$1
    if [[ -z ${CACHE_VAL[$key]+x} ]]; then
        echo "  miss: $key"
        return 1
    fi
    if (( EPOCHSECONDS > CACHE_EXP[$key] )); then
        unset "CACHE_VAL[$key]"
        unset "CACHE_EXP[$key]"
        echo "  expired: $key"
        return 1
    fi
    echo "  hit: $key = ${CACHE_VAL[$key]}"
}

cache_gc() {
    local removed=0
    for k in ${(k)CACHE_VAL}; do
        if (( EPOCHSECONDS > CACHE_EXP[$k] )); then
            unset "CACHE_VAL[$k]"
            unset "CACHE_EXP[$k]"
            (( removed++ ))
        fi
    done
    echo "  gc: removed $removed expired entries"
}

cache_dump() {
    echo "  active entries: ${#CACHE_VAL[@]}"
    for k in ${(ko)CACHE_VAL}; do
        local ttl_left=$(( CACHE_EXP[$k] - EPOCHSECONDS ))
        printf "    %-10s = %-8s (expires in %ds)\n" "$k" "${CACHE_VAL[$k]}" $ttl_left
    done
}

echo "── populate cache with various TTLs ──"
cache_set short_lived A 1
cache_set medium_lived B 5
cache_set long_lived C 100
cache_set very_short D 1

echo "── immediate dump ──"
cache_dump

echo "── sleep 2s ──"
sleep 2

echo "── after sleep, fetch ──"
cache_get short_lived
cache_get medium_lived
cache_get long_lived
cache_get very_short

echo "── gc ──"
cache_gc
cache_dump

echo "── sleep 4 more, then dump ──"
sleep 4
cache_gc
cache_dump

echo "── refresh medium ──"
cache_set medium_lived B_refreshed 60
cache_dump

echo "── miss key ──"
cache_get nonexistent

# === ztest assertions ===
# After all the operations above: long_lived + medium_lived(refreshed) survive.
zassert_eq "${#CACHE_VAL[@]}" 2                "two cache entries survive"
zassert_eq "${CACHE_VAL[long_lived]}" "C"      "long_lived value"
zassert_eq "${CACHE_VAL[medium_lived]}" "B_refreshed"  "medium_lived refreshed"
# Now test cache_set/cache_get directly
cache_set test_key Z 100 > /dev/null
zassert_eq "${CACHE_VAL[test_key]}" "Z"        "fresh set lands"
zassert_contains "$(cache_get test_key)" "hit" "fresh get is a hit"
zassert_contains "$(cache_get bogus)"    "miss" "missing key reports miss"
ztest_run
