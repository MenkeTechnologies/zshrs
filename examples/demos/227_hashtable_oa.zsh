#!/usr/bin/env zshrs
# Open-addressing hash table implementation in pure zsh.

typeset -a HT_KEYS HT_VALS
typeset -i HT_SIZE=16
typeset -i HT_COUNT=0

ht_hash() {
    local s=$1 h=0 i ch
    for ((i=1; i<=${#s}; i++)); do
        ch=${s[i]}
        h=$(( (h * 31 + $(printf "%d" "'$ch")) & 0x7FFFFFFF ))
    done
    echo $(( h % HT_SIZE + 1 ))
}

ht_init() {
    HT_KEYS=()
    HT_VALS=()
    local i
    for ((i=0; i<HT_SIZE; i++)); do
        HT_KEYS+=("")
        HT_VALS+=("")
    done
    HT_COUNT=0
}

ht_set() {
    local k=$1 v=$2
    local idx=$(ht_hash "$k")
    local probes=0
    while (( probes < HT_SIZE )); do
        if [[ -z ${HT_KEYS[idx]} || ${HT_KEYS[idx]} == $k ]]; then
            local was_new=0
            [[ -z ${HT_KEYS[idx]} ]] && was_new=1
            HT_KEYS[idx]=$k
            HT_VALS[idx]=$v
            (( was_new )) && (( HT_COUNT++ ))
            return
        fi
        (( idx = idx % HT_SIZE + 1 ))
        (( probes++ ))
    done
    echo "table full"
}

ht_get() {
    local k=$1
    local idx=$(ht_hash "$k")
    local probes=0
    while (( probes < HT_SIZE )); do
        if [[ -z ${HT_KEYS[idx]} ]]; then
            echo "(none)"
            return 1
        fi
        if [[ ${HT_KEYS[idx]} == $k ]]; then
            echo "${HT_VALS[idx]}"
            return 0
        fi
        (( idx = idx % HT_SIZE + 1 ))
        (( probes++ ))
    done
    echo "(none)"
    return 1
}

ht_dump() {
    echo "  count=$HT_COUNT size=$HT_SIZE load=$(( HT_COUNT * 100 / HT_SIZE ))%"
    local i
    for ((i=1; i<=HT_SIZE; i++)); do
        if [[ -n ${HT_KEYS[i]} ]]; then
            printf "  [%2d] %s = %s\n" $i "${HT_KEYS[i]}" "${HT_VALS[i]}"
        fi
    done
}

echo "── init ──"
ht_init
ht_dump

echo
echo "── set 8 entries ──"
ht_set apple 100
ht_set banana 50
ht_set cherry 200
ht_set date 80
ht_set elderberry 150
ht_set fig 30
ht_set grape 120
ht_set honeydew 90
ht_dump

echo
echo "── lookups ──"
for k in apple banana cherry nonexistent grape; do
    echo "  $k → $(ht_get $k)"
done

echo
echo "── update existing ──"
ht_set apple 999
echo "after update: apple = $(ht_get apple)"
echo "count still: $HT_COUNT"

echo
echo "── hash distribution ──"
for k in apple banana cherry date elderberry fig grape honeydew; do
    h=$(ht_hash "$k")
    printf "  %-12s → bucket %d\n" $k $h
done

# === ztest assertions ===
zassert_eq "$HT_COUNT" 8                "8 unique keys inserted"
zassert_eq "$HT_SIZE"  16               "table size"
zassert_eq "$(ht_get apple)"   999      "apple updated to 999"
zassert_eq "$(ht_get banana)"  50       "banana lookup"
zassert_eq "$(ht_get cherry)"  200      "cherry lookup"
zassert_eq "$(ht_get grape)"   120      "grape lookup"
zassert_eq "$(ht_get nonexistent)" "(none)" "missing key returns (none)"
# hash determinism + range
h=$(ht_hash apple)
zassert_ge "$h" 1  "hash >= 1"
zassert_le "$h" 16 "hash <= HT_SIZE"
# update did not bump count
zassert_eq "$HT_COUNT" 8 "count unchanged after update"
ztest_run
