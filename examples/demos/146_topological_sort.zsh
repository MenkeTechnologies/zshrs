#!/usr/bin/env zshrs
# Topological sort over a DAG of dependencies.

typeset -A DEPS

dep() {
    local target=$1
    shift
    DEPS[$target]="$*"
}

# Build a typical build-system dep DAG:
dep app   "main lib utils"
dep main  "lib"
dep lib   "utils config"
dep utils ""
dep config ""

echo "── dependencies ──"
for k in ${(ko)DEPS}; do
    echo "  $k ← ${DEPS[$k]:-(none)}"
done

# Compute in-degrees.
toposort() {
    local -A indeg
    local node
    for node in ${(k)DEPS}; do
        indeg[$node]=0
    done
    for node in ${(k)DEPS}; do
        for dep in ${(s/ /)DEPS[$node]}; do
            (( indeg[$node]++ ))
        done
    done

    # Queue of in-deg=0 nodes (leaves of dep graph).
    local -a queue=()
    for node in ${(k)indeg}; do
        if (( indeg[$node] == 0 )); then
            queue+=($node)
        fi
    done

    local order=""
    while (( ${#queue[@]} > 0 )); do
        local node=${queue[1]}
        queue=("${queue[@]:1}")
        order+="$node "
        # Find nodes that depend ON this node — they lose 1 in-degree.
        for downstream in ${(k)DEPS}; do
            for d in ${(s/ /)DEPS[$downstream]}; do
                if [[ $d == $node ]]; then
                    (( indeg[$downstream]-- ))
                    if (( indeg[$downstream] == 0 )); then
                        queue+=($downstream)
                    fi
                fi
            done
        done
    done
    echo "${order% }"
}

echo "── build order ──"
toposort

echo "── linear chain example ──"
DEPS=()
dep z y
dep y x
dep x w
dep w ""

echo "linear order:"
toposort

echo "── parallel branches ──"
DEPS=()
dep final "branch_a branch_b"
dep branch_a "a1 a2"
dep branch_b "b1 b2"
dep a1 ""
dep a2 ""
dep b1 ""
dep b2 ""

echo "parallel order:"
toposort
