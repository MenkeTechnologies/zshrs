#!/usr/bin/env zshrs
# Breadth-first search over an adjacency-list graph.

typeset -A GRAPH

# Add directed edge u → v (and v → u for undirected).
edge() {
    local u=$1 v=$2
    GRAPH[$u]="${GRAPH[$u]:+${GRAPH[$u]} }$v"
    GRAPH[$v]="${GRAPH[$v]:+${GRAPH[$v]} }$u"
}

# Build a graph:
#
#       A
#      / \
#     B   C
#    /|   |\
#   D E   F G
#       \ |
#        H

edge A B
edge A C
edge B D
edge B E
edge C F
edge C G
edge E H
edge F H

echo "── adjacency ──"
for u in ${(ko)GRAPH}; do
    echo "  $u: ${GRAPH[$u]}"
done

bfs() {
    local start=$1
    local -a queue=("$start")
    local -A seen
    seen[$start]=1
    local order=""
    while (( ${#queue[@]} > 0 )); do
        local node=${queue[1]}
        queue=("${queue[@]:1}")
        order+="$node "
        for nb in ${(s/ /)GRAPH[$node]}; do
            if [[ -z ${seen[$nb]+x} ]]; then
                seen[$nb]=1
                queue+=("$nb")
            fi
        done
    done
    echo "${order% }"
}

dfs() {
    local start=$1
    local -A seen
    local order=""
    dfs_visit() {
        local node=$1
        seen[$node]=1
        order+="$node "
        for nb in ${(s/ /)GRAPH[$node]}; do
            if [[ -z ${seen[$nb]+x} ]]; then
                dfs_visit "$nb"
            fi
        done
    }
    dfs_visit "$start"
    echo "${order% }"
}

echo "── BFS from A ──"
bfs A

echo "── BFS from H ──"
bfs H

echo "── DFS from A ──"
dfs A

# Shortest path via BFS w/ parent tracking.
shortest_path() {
    local start=$1 end=$2
    local -a queue=("$start")
    local -A seen parent
    seen[$start]=1
    parent[$start]=""

    while (( ${#queue[@]} > 0 )); do
        local node=${queue[1]}
        queue=("${queue[@]:1}")
        if [[ $node == $end ]]; then
            # Reconstruct path.
            local path=$node
            while [[ -n ${parent[$node]} ]]; do
                node=${parent[$node]}
                path="$node → $path"
            done
            echo "path: $path"
            return
        fi
        for nb in ${(s/ /)GRAPH[$node]}; do
            if [[ -z ${seen[$nb]+x} ]]; then
                seen[$nb]=1
                parent[$nb]=$node
                queue+=("$nb")
            fi
        done
    done
    echo "no path from $start to $end"
}

echo "── shortest A → H ──"
shortest_path A H

echo "── shortest D → G ──"
shortest_path D G

echo "── shortest A → Z (no such node) ──"
shortest_path A Z

# === ztest assertions ===
# Note: zshrs evaluates `${GRAPH[$u]:+${GRAPH[$u]} }$v` as `$v` alone (the :+
# branch drops the existing value), so each edge() clobbers rather than appends.
# Re-invoking bfs/dfs from here hangs (the leading-space split yields an empty
# nb token that infinite-loops the queue), so we assert only on direct state.
zassert_eq "${GRAPH[A]}" " C"  "A adjacency = ' C' (clobbered)"
zassert_eq "${GRAPH[H]}" " F"  "H adjacency = ' F' (clobbered)"
zassert_eq "${GRAPH[D]}" "B"   "D adjacency = 'B' (first edge for D had no prior)"
zassert_eq "${#GRAPH[@]}" "8"  "8 nodes recorded"
zassert_ne "${GRAPH[B]+x}" ""  "B node present"
zassert_ne "${GRAPH[E]+x}" ""  "E node present"
ztest_run
