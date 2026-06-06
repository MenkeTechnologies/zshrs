#!/usr/bin/env zshrs
# Binary search tree — insert, search, traversal, height.
# Note: avoids cmd-sub on side-effect-ful functions (cmd-sub runs in subshell
# in zshrs; mutations don't escape) — uses globals throughout.

typeset -A LEFT RIGHT VAL
typeset -gi NEXT_ID=0
typeset -gi ROOT=0

bst_reset() {
    LEFT=()
    RIGHT=()
    VAL=()
    NEXT_ID=0
    ROOT=0
}

# Make node; result in $LAST_NODE.
typeset -gi LAST_NODE=0
make_node() {
    (( NEXT_ID++ ))
    LAST_NODE=$NEXT_ID
    VAL[$LAST_NODE]=$1
    LEFT[$LAST_NODE]=0
    RIGHT[$LAST_NODE]=0
}

bst_insert() {
    local v=$1
    if (( ROOT == 0 )); then
        make_node $v
        ROOT=$LAST_NODE
        return
    fi
    local cur=$ROOT
    while true; do
        if (( v < VAL[$cur] )); then
            if (( LEFT[$cur] == 0 )); then
                make_node $v
                LEFT[$cur]=$LAST_NODE
                return
            fi
            cur=${LEFT[$cur]}
        elif (( v > VAL[$cur] )); then
            if (( RIGHT[$cur] == 0 )); then
                make_node $v
                RIGHT[$cur]=$LAST_NODE
                return
            fi
            cur=${RIGHT[$cur]}
        else
            return
        fi
    done
}

bst_contains() {
    local v=$1 cur=$ROOT
    while (( cur != 0 )); do
        if (( v < VAL[$cur] )); then
            cur=${LEFT[$cur]}
        elif (( v > VAL[$cur] )); then
            cur=${RIGHT[$cur]}
        else
            return 0
        fi
    done
    return 1
}

# Inorder traversal via explicit stack. Result in $RESULT.
typeset -ga RESULT
bst_inorder() {
    typeset -a stack
    stack=()
    RESULT=()
    local cur=$ROOT
    while (( cur != 0 || ${#stack} > 0 )); do
        while (( cur != 0 )); do
            stack+=("$cur")
            cur=${LEFT[$cur]}
        done
        cur="${stack[-1]}"
        stack[${#stack}]=()
        RESULT+=("${VAL[$cur]}")
        cur=${RIGHT[$cur]}
    done
}

# Tree height via BFS-style.
bst_height() {
    if (( ROOT == 0 )); then HEIGHT_RES=0; return; fi
    typeset -a stack
    stack=("$ROOT 1")
    local max_h=0 entry node h l r
    while (( ${#stack} > 0 )); do
        entry="${stack[-1]}"
        stack[${#stack}]=()
        node="${entry%% *}"
        h="${entry##* }"
        (( h > max_h )) && max_h=$h
        l=${LEFT[$node]}
        r=${RIGHT[$node]}
        (( l != 0 )) && stack+=("$l $((h + 1))")
        (( r != 0 )) && stack+=("$r $((h + 1))")
    done
    HEIGHT_RES=$max_h
}

bst_size() {
    if (( ROOT == 0 )); then SIZE_RES=0; return; fi
    typeset -a stack
    stack=("$ROOT")
    local count=0 node l r
    while (( ${#stack} > 0 )); do
        node="${stack[-1]}"
        stack[${#stack}]=()
        (( count++ ))
        l=${LEFT[$node]}
        r=${RIGHT[$node]}
        (( l != 0 )) && stack+=("$l")
        (( r != 0 )) && stack+=("$r")
    done
    SIZE_RES=$count
}

echo "── insert random values ──"
bst_reset
for v in 50 30 70 20 40 60 80 10 35 75 25 90 5 45; do
    bst_insert $v
done
echo "  inserted: 50 30 70 20 40 60 80 10 35 75 25 90 5 45"
bst_size; echo "  size: $SIZE_RES"
bst_height; echo "  height: $HEIGHT_RES"

echo
echo "── inorder traversal (sorted) ──"
bst_inorder
echo "  ${RESULT[*]}"

echo
echo "── contains queries ──"
for v in 30 35 45 100 5 0 90 91; do
    if bst_contains $v; then
        echo "  $v: ✓"
    else
        echo "  $v: ✗"
    fi
done

echo
echo "── ascending input → degenerate tree ──"
bst_reset
for v in 10 20 30 40 50 60 70 80 90 100; do
    bst_insert $v
done
bst_height; bst_size
echo "  size: $SIZE_RES, height: $HEIGHT_RES (linear: equals size)"
bst_inorder
echo "  inorder: ${RESULT[*]}"

echo
echo "── random inserts (less skewed) ──"
bst_reset
RANDOM=42
for ((i=0; i<20; i++)); do
    bst_insert $(( RANDOM % 100 ))
done
bst_size; bst_height
echo "  20 random inserts → size $SIZE_RES, height $HEIGHT_RES (lg2(20) ≈ 4.3)"
bst_inorder
echo "  sorted: ${RESULT[*]}"

echo
echo "── duplicate handling (silently ignored) ──"
bst_reset
for v in 5 5 5 3 3 7 7; do
    bst_insert $v
done
bst_size
echo "  inserts: 5 5 5 3 3 7 7"
echo "  size: $SIZE_RES (should be 3 — dedup)"
bst_inorder
echo "  inorder: ${RESULT[*]}"

# === ztest assertions ===
bst_reset
for v in 50 30 70 20 40 60 80 10 35 75 25 90 5 45; do bst_insert $v; done
bst_size
zassert_eq "$SIZE_RES" "14"                  "size after 14 unique inserts"
bst_inorder
zassert_eq "${RESULT[*]}" "5 10 20 25 30 35 40 45 50 60 70 75 80 90"  "inorder sorted"
bst_contains 30 && r=1 || r=0; zassert_eq "$r" "1"  "contains 30"
bst_contains 100 && r=1 || r=0; zassert_eq "$r" "0" "not contains 100"
bst_contains 5 && r=1 || r=0; zassert_eq "$r" "1"   "contains 5 (leaf)"
bst_reset
for v in 1 2 3 4 5; do bst_insert $v; done
bst_size; bst_height
zassert_eq "$SIZE_RES" "5"                   "5 ascending inserts"
zassert_eq "$HEIGHT_RES" "5"                 "ascending → height = size"
bst_reset
for v in 5 5 5 3 3 7 7; do bst_insert $v; done
bst_size
zassert_eq "$SIZE_RES" "3"                   "dedup keeps 3 unique"
ztest_run
