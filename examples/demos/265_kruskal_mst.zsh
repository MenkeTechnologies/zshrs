#!/usr/bin/env zshrs
# Kruskal's MST — union-find + sort-by-weight.

typeset -A PARENT RANK

uf_init() {
    local node
    for node in "$@"; do
        PARENT[$node]=$node
        RANK[$node]=0
    done
}

uf_find() {
    local x=$1
    while [[ ${PARENT[$x]} != $x ]]; do
        PARENT[$x]=${PARENT[${PARENT[$x]}]}  # path compression
        x=${PARENT[$x]}
    done
    echo $x
}

uf_union() {
    local x=$1 y=$2 rx ry
    rx=$(uf_find $x)
    ry=$(uf_find $y)
    [[ $rx == $ry ]] && return 1
    if (( RANK[$rx] < RANK[$ry] )); then
        PARENT[$rx]=$ry
    elif (( RANK[$rx] > RANK[$ry] )); then
        PARENT[$ry]=$rx
    else
        PARENT[$ry]=$rx
        (( RANK[$rx]++ ))
    fi
    return 0
}

# Edges as "weight u v".
edges=(
    "1 A B"
    "4 A C"
    "3 B C"
    "2 B D"
    "5 C D"
    "6 C E"
    "7 D E"
    "8 D F"
    "9 E F"
    "10 A F"
)

nodes=(A B C D E F)

echo "── input graph ──"
echo "  nodes: ${nodes[@]}"
echo "  edges (weight, u-v):"
for e in "${edges[@]}"; do
    set -- ${=e}
    printf "    %2d  %s—%s\n" $1 $2 $3
done

echo
echo "── sort edges by weight (Kruskal step 1) ──"
sorted=( "${(@n)edges}" )
for e in "${sorted[@]}"; do
    set -- ${=e}
    printf "    %2d  %s—%s\n" $1 $2 $3
done

echo
echo "── MST construction ──"
uf_init "${nodes[@]}"
mst=()
total=0
for e in "${sorted[@]}"; do
    set -- ${=e}
    w=$1; u=$2; v=$3
    if uf_union $u $v; then
        printf "  + accept %s—%s (w=%d)\n" $u $v $w
        mst+=("$w $u $v")
        (( total += w ))
    else
        printf "  - skip   %s—%s (w=%d) [forms cycle]\n" $u $v $w
    fi
done

echo
echo "── MST result ──"
echo "  ${#mst} edges, total weight: $total"
for e in "${mst[@]}"; do
    set -- ${=e}
    printf "    %s—%s (w=%d)\n" $2 $3 $1
done

echo
echo "── verify: connected components ──"
typeset -A comp
for n in "${nodes[@]}"; do
    root=$(uf_find $n)
    (( comp[$root]++ ))
done
echo "  components: ${#comp}"
if (( ${#comp} == 1 )); then
    echo "  ✓ spanning tree (all nodes connected)"
fi

echo
echo "── 2nd test: disconnected graph ──"
typeset -A PARENT RANK
PARENT=()
RANK=()
uf_init A B C D E F G
# Edges only in {A,B,C} and {E,F,G}, leaving D isolated.
edges2=("1 A B" "2 B C" "3 E F" "4 F G")
for e in "${edges2[@]}"; do
    set -- ${=e}
    uf_union $2 $3 > /dev/null
done
typeset -A comp2
for n in A B C D E F G; do
    r=$(uf_find $n)
    (( comp2[$r]++ ))
done
echo "  components after partial MST: ${#comp2}"
for k in "${(@k)comp2}"; do
    printf "    root %s: %d nodes\n" $k ${comp2[$k]}
done

# === ztest assertions ===
zassert_eq "${#mst}"   5    "MST has |V|-1 edges"
zassert_eq "$total"    20   "MST total weight"
zassert_eq "${#comp}"  1    "single component after Kruskal"
zassert_eq "${#comp2}" 3    "disconnected graph: 3 components (ABC | D | EFG)"
zassert_eq "${comp2[D]}" 1  "isolated D forms component of size 1"
zassert_eq "${#nodes}" 6    "6 nodes"
zassert_eq "${#edges}" 10   "10 input edges"
zassert_eq "${sorted[1]}" "1 A B"  "sort -n picks lightest edge first"
ztest_run
