#!/usr/bin/env zshrs
# Bellman-Ford — single-source shortest paths w/ negative edges.

# Edges: each "u v w".
edges=(
    "S A 4"
    "S B 5"
    "S C 3"
    "A B -3"
    "A C 2"
    "B D 4"
    "C B 1"
    "C D 7"
    "D E 2"
    "B E 8"
)

nodes=(S A B C D E)
N=${#nodes}
INF=999999

typeset -A dist prev
for n in "${nodes[@]}"; do
    dist[$n]=$INF
    prev[$n]="-"
done
dist[S]=0

echo "── input ──"
echo "  edges:"
for e in "${edges[@]}"; do
    set -- ${=e}
    printf "    %s → %s (w=%d)\n" $1 $2 $3
done
echo "  source: S"

echo
echo "── relaxation (N-1 = $((N-1)) iterations) ──"
for ((iter=1; iter<N; iter++)); do
    changed=0
    for e in "${edges[@]}"; do
        set -- ${=e}
        u=$1; v=$2; w=$3
        du=${dist[$u]}
        if (( du < INF )); then
            nv=$(( du + w ))
            dv=${dist[$v]}
            if (( nv < dv )); then
                dist[$v]=$nv
                prev[$v]=$u
                changed=1
            fi
        fi
    done
    if (( changed )); then
        printf "  iter %d: " $iter
        for n in "${nodes[@]}"; do
            d=${dist[$n]}
            if (( d == INF )); then
                printf "%s=∞ " $n
            else
                printf "%s=%d " $n $d
            fi
        done
        echo
    else
        echo "  iter $iter: no change (converged)"
        break
    fi
done

echo
echo "── final distances from S ──"
for n in "${nodes[@]}"; do
    d=${dist[$n]}
    if (( d == INF )); then
        printf "  S → %s : ∞\n" $n
    else
        # Reconstruct path.
        path=$n
        cur=$n
        while [[ ${prev[$cur]} != "-" ]]; do
            cur=${prev[$cur]}
            path="$cur→$path"
        done
        printf "  S → %s : %3d   via %s\n" $n $d "$path"
    fi
done

echo
echo "── neg-cycle detection (Nth relax) ──"
neg_cycle=0
for e in "${edges[@]}"; do
    set -- ${=e}
    u=$1; v=$2; w=$3
    du=${dist[$u]}
    dv=${dist[$v]}
    if (( du < INF && du + w < dv )); then
        echo "  ✗ negative cycle via edge $u → $v (w=$w)"
        neg_cycle=1
    fi
done
(( ! neg_cycle )) && echo "  ✓ no negative cycle reachable from S"

echo
echo "── graph w/ negative cycle test ──"
neg_edges=(
    "S A 1"
    "A B -3"
    "B A 1"
)
typeset -A dist2
for n in S A B; do dist2[$n]=$INF; done
dist2[S]=0
for ((iter=1; iter<3; iter++)); do
    for e in "${neg_edges[@]}"; do
        set -- ${=e}
        if (( dist2[$1] < INF )); then
            n=$(( dist2[$1] + $3 ))
            if (( n < dist2[$2] )); then
                dist2[$2]=$n
            fi
        fi
    done
done
# 3rd relax detect.
detected=0
for e in "${neg_edges[@]}"; do
    set -- ${=e}
    if (( dist2[$1] < INF && dist2[$1] + $3 < dist2[$2] )); then
        echo "  detected: edge $1→$2 relaxes after V-1 → neg cycle"
        detected=1
        break
    fi
done
(( ! detected )) && echo "  no neg cycle"

# === ztest assertions ===
zassert_eq "$N"           6 "6 nodes"
zassert_eq "${dist[S]}"   0 "d(S,S) = 0"
zassert_eq "${dist[A]}"   4 "d(S,A) = 4"
zassert_eq "${dist[B]}"   1 "d(S,B) = 1 (via A, -3 edge)"
zassert_eq "${dist[C]}"   3 "d(S,C) = 3 (direct)"
zassert_eq "${dist[D]}"   5 "d(S,D) = 5 (via A,B)"
zassert_eq "${dist[E]}"   7 "d(S,E) = 7 (via A,B,D)"
zassert_eq "${prev[B]}"   A "prev[B] = A"
zassert_eq "$neg_cycle"   0 "no neg cycle in main graph"
zassert_eq "$detected"    1 "negative cycle detected in test graph"
ztest_run
