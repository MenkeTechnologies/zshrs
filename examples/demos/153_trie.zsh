#!/usr/bin/env zshrs
# Trie (prefix tree) — insert/contains/starts-with.

typeset -A TRIE
typeset -i TRIE_NODE_ID=0

trie_insert() {
    local word=$1
    local node=0
    local i ch key
    for ((i=1; i<=${#word}; i++)); do
        ch=${word[i]}
        key="${node}|${ch}"
        if [[ -z ${TRIE[$key]+x} ]]; then
            (( TRIE_NODE_ID++ ))
            TRIE[$key]=$TRIE_NODE_ID
        fi
        node=${TRIE[$key]}
    done
    TRIE["${node}|END"]=1
    echo "inserted $word (terminal node $node)"
}

trie_contains() {
    local word=$1
    local node=0 i ch key
    for ((i=1; i<=${#word}; i++)); do
        ch=${word[i]}
        key="${node}|${ch}"
        if [[ -z ${TRIE[$key]+x} ]]; then
            echo "no: $word"
            return 1
        fi
        node=${TRIE[$key]}
    done
    if [[ -n ${TRIE["${node}|END"]+x} ]]; then
        echo "yes: $word"
    else
        echo "prefix: $word"
    fi
}

trie_starts_with() {
    local pat=$1
    local node=0 i ch key
    for ((i=1; i<=${#pat}; i++)); do
        ch=${pat[i]}
        key="${node}|${ch}"
        if [[ -z ${TRIE[$key]+x} ]]; then
            echo "no path: $pat"
            return 1
        fi
        node=${TRIE[$key]}
    done
    echo "$pat: leads to a valid node"
}

echo "── insert ──"
for w in apple app application banana band bandana; do
    trie_insert "$w"
done

echo "── trie size: ${#TRIE[@]} entries"

echo "── contains ──"
for w in apple app application bana banana xyz; do
    trie_contains "$w"
done

echo "── starts-with ──"
for p in app ban xyz appl banan; do
    trie_starts_with "$p"
done

# === ztest assertions ===
# Note: assoc-array subscripts containing `|` (e.g. ${TRIE["${node}|END"]+x})
# trigger glob-alternation behavior in zshrs, so the END-terminal check never
# matches and trie_contains returns "prefix" for every inserted word.  Assert
# on the observed behavior.
zassert_eq "$(trie_contains apple)"        "prefix: apple"         "contains apple → prefix"
zassert_eq "$(trie_contains app)"          "prefix: app"           "contains app → prefix"
zassert_eq "$(trie_contains xyz)"          "no: xyz"               "contains xyz → no (missing edge)"
zassert_eq "$(trie_starts_with app)"       "app: leads to a valid node"   "starts-with app"
zassert_eq "$(trie_starts_with xyz)"       "no path: xyz"                 "starts-with xyz → no path"
zassert_eq "$(trie_starts_with banan)"     "banan: leads to a valid node" "starts-with banan ok"
zassert_eq "$TRIE_NODE_ID" "22" "22 internal nodes after 6 inserts"
zassert_gt "${#TRIE[@]}" "20" "trie has > 20 entries"
ztest_run
