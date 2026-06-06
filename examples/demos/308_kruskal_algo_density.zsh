#!/usr/bin/env zshrs
# Graph density study — generate random graphs, run Kruskal, compare MST weight.

typeset -A PARENT_UF RANK_UF
typeset -ga EDGES MST_EDGES

uf_reset() {
    PARENT_UF=()
    RANK_UF=()
    local n=$1 i
    for ((i=1; i<=n; i++)); do
        PARENT_UF[$i]=$i
        RANK_UF[$i]=0
    done
}

uf_find_inline() {
    # Inline find returning result via $result variable to avoid cmd-sub.
    local x=$1 p
    while true; do
        p=${PARENT_UF[$x]}
        [[ $p == $x ]] && break
        PARENT_UF[$x]=${PARENT_UF[$p]}
        x=$p
    done
    result=$x
}

uf_union() {
    local x=$1 y=$2
    uf_find_inline $x
    local rx=$result
    uf_find_inline $y
    local ry=$result
    [[ $rx == $ry ]] && return 1
    if (( RANK_UF[$rx] < RANK_UF[$ry] )); then
        PARENT_UF[$rx]=$ry
    elif (( RANK_UF[$rx] > RANK_UF[$ry] )); then
        PARENT_UF[$ry]=$rx
    else
        PARENT_UF[$ry]=$rx
        (( RANK_UF[$rx]++ ))
    fi
    return 0
}

# Generate edges for N nodes at given density (% of possible edges).
gen_graph() {
    local n=$1 density=$2
    EDGES=()
    local i j w
    for ((i=1; i<=n; i++)); do
        for ((j=i+1; j<=n; j++)); do
            if (( RANDOM % 100 < density )); then
                w=$(( RANDOM % 20 + 1 ))
                EDGES+=("$w $i $j")
            fi
        done
    done
}

kruskal_weight() {
    local n=$1
    uf_reset $n
    MST_EDGES=()
    local total=0 e w u v
    local -a sorted
    sorted=( "${(@n)EDGES}" )
    for e in "${sorted[@]}"; do
        # Parse w u v from "w u v" string without set --.
        w="${e%% *}"
        local rest="${e#* }"
        u="${rest%% *}"
        v="${rest#* }"
        if uf_union $u $v; then
            MST_EDGES+=("$e")
            (( total += w ))
        fi
    done
    result=$total
}

count_components() {
    local n=$1 i
    typeset -A roots
    roots=()
    for ((i=1; i<=n; i++)); do
        uf_find_inline $i
        (( roots[$result]++ ))
    done
    result=${#roots}
}

RANDOM=42

echo "── density study (N=10) ──"
echo "  density   edges     MST_weight   components"
for d in 20 40 60 80 100; do
    RANDOM=42
    gen_graph 10 $d
    e=${#EDGES}
    kruskal_weight 10
    w=$result
    count_components 10
    comps=$result
    printf "  %3d%%      %2d edges    %3d           %d\n" $d $e $w $comps
done

echo
echo "── single larger graph (N=20, density=30%) ──"
RANDOM=42
gen_graph 20 30
echo "  nodes: 20"
echo "  edges: ${#EDGES}"
kruskal_weight 20
echo "  MST weight: $result"
echo "  MST edges:  ${#MST_EDGES}"
count_components 20
echo "  components after MST: $result"

echo
echo "── edge weight histogram ──"
typeset -A hist
for e in "${EDGES[@]}"; do
    w="${e%% *}"
    (( hist[$w]++ ))
done
for w in {1..20}; do
    c=${hist[$w]:-0}
    bar=""
    for ((b=0; b<c; b++)); do bar+="█"; done
    printf "  w=%2d: %2d %s\n" $w $c "$bar"
done

echo
echo "── small dense graph: 5 nodes, all edges ──"
RANDOM=42
gen_graph 5 100
echo "  nodes: 5   edges: ${#EDGES} (out of $(( 5*4/2 )) possible)"
kruskal_weight 5
echo "  MST weight: $result"
echo "  MST edges:"
for e in "${MST_EDGES[@]}"; do
    w="${e%% *}"
    rest="${e#* }"
    u="${rest%% *}"
    v="${rest#* }"
    printf "    %s—%s (w=%s)\n" $u $v $w
done

echo
echo "── density vs MST cost (avg over 5 trials each) ──"
for d in 30 50 70; do
    local sum=0 trials=0
    for trial in 1 2 3 4 5; do
        RANDOM=$(( trial * 7 + 1 ))
        gen_graph 10 $d
        if (( ${#EDGES} > 0 )); then
            kruskal_weight 10
            (( sum += result ))
            (( trials++ ))
        fi
    done
    if (( trials > 0 )); then
        printf "  density=%2d%% : avg MST = %d (over %d trials)\n" $d $((sum / trials)) $trials
    fi
done

# === ztest assertions ===
# Union-find: connect, find, components
uf_reset 5
zassert_eq "${#PARENT_UF}" "5"           "uf_reset creates 5 nodes"
uf_union 1 2
uf_union 3 4
count_components 5
zassert_eq "$result" "3"                 "after 2 unions of 5 nodes, 3 components"
uf_union 1 3
count_components 5
zassert_eq "$result" "2"                 "after 3rd union, 2 components"
# Kruskal on a small fixed graph
EDGES=("1 1 2" "2 2 3" "3 3 4" "4 1 4" "5 1 3")
kruskal_weight 4
zassert_eq "$result" "6"                 "MST(1+2+3)=6 on 4-node chain"
zassert_eq "${#MST_EDGES}" "3"           "MST has n-1=3 edges"
# Deterministic: same seed reproduces
RANDOM=42
gen_graph 10 100
n1=${#EDGES}
RANDOM=42
gen_graph 10 100
n2=${#EDGES}
zassert_eq "$n1" "$n2"                   "seed 42 reproduces"
ztest_run
