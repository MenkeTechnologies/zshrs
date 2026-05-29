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
