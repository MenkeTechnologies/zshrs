#!/usr/bin/env zshrs
# Floyd-Warshall — all-pairs shortest paths, O(V^3) DP.

# Nodes A..E, indexed 1..5.
NODES=(A B C D E)
N=${#NODES}
INF=999999

# Distance matrix d[i][j] indexed (i-1)*N+j.
typeset -a D

# Initialize: diag=0, else INF.
for ((i=1; i<=N; i++)); do
    for ((j=1; j<=N; j++)); do
        idx=$(( (i-1)*N + j ))
        if (( i == j )); then
            D[idx]=0
        else
            D[idx]=$INF
        fi
    done
done

# Edges as "from to weight".
set_edge() {
    local fi=$1 ti=$2 w=$3
    local idx=$(( (fi-1)*N + ti ))
    D[idx]=$w
}

# Convert name → index.
node_idx() {
    local n=$1 i
    for ((i=1; i<=N; i++)); do
        [[ ${NODES[i]} == $n ]] && { echo $i; return; }
    done
}

edges=(
    "A B 3"
    "A C 8"
    "A E -4"
    "B D 1"
    "B E 7"
    "C B 4"
    "D A 2"
    "D C -5"
    "E D 6"
)

echo "── input directed weighted edges ──"
for e in "${edges[@]}"; do
    set -- ${=e}
    fi=$(node_idx $1)
    ti=$(node_idx $2)
    set_edge $fi $ti $3
    printf "  %s → %s (w=%d)\n" $1 $2 $3
done

print_matrix() {
    printf "       "
    for n in "${NODES[@]}"; do printf "%7s" $n; done
    echo
    local i j idx v
    for ((i=1; i<=N; i++)); do
        printf "  %s |  " "${NODES[i]}"
        for ((j=1; j<=N; j++)); do
            idx=$(( (i-1)*N + j ))
            v=${D[idx]}
            if (( v == INF )); then
                printf "    ∞ "
            else
                printf "%6d " $v
            fi
        done
        echo
    done
}

echo
echo "── initial distance matrix ──"
print_matrix

echo
echo "── Floyd-Warshall iterations (showing k=A,B,C,D,E) ──"
for ((k=1; k<=N; k++)); do
    for ((i=1; i<=N; i++)); do
        for ((j=1; j<=N; j++)); do
            idx_ij=$(( (i-1)*N + j ))
            idx_ik=$(( (i-1)*N + k ))
            idx_kj=$(( (k-1)*N + j ))
            v_ik=${D[idx_ik]}
            v_kj=${D[idx_kj]}
            if (( v_ik < INF && v_kj < INF )); then
                via=$(( v_ik + v_kj ))
                cur=${D[idx_ij]}
                if (( via < cur )); then
                    D[idx_ij]=$via
                fi
            fi
        done
    done
    echo
    echo "  after k=${NODES[k]}:"
    print_matrix
done

echo
echo "── path lookups ──"
queries=("A C" "A D" "B A" "C E" "E A" "D B")
for q in "${queries[@]}"; do
    set -- ${=q}
    src=$(node_idx $1)
    dst=$(node_idx $2)
    idx=$(( (src-1)*N + dst ))
    v=${D[idx]}
    if (( v == INF )); then
        printf "  d(%s → %s) = ∞ (unreachable)\n" $1 $2
    else
        printf "  d(%s → %s) = %d\n" $1 $2 $v
    fi
done

echo
echo "── detect negative cycle ──"
neg_cyc=0
for ((i=1; i<=N; i++)); do
    idx=$(( (i-1)*N + i ))
    if (( D[idx] < 0 )); then
        echo "  ✗ negative cycle through ${NODES[i]} (d=${D[idx]})"
        neg_cyc=1
    fi
done
(( ! neg_cyc )) && echo "  ✓ no negative cycle"
