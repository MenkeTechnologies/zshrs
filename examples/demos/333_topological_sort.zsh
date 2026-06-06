#!/usr/bin/env zshrs
# Topological sort — Kahn's algorithm + cycle detection.

# DEPS[task] = "depends on these tasks"
typeset -A DEPS

ts_clear() { DEPS=(); }

ts_add() {
    local task=$1
    shift
    DEPS[$task]="$*"
}

# Kahn's algo: repeatedly emit nodes with in-degree 0.
topo_sort() {
    typeset -A indeg
    # Collect all nodes.
    typeset -A all_nodes
    local t dep
    for t in "${(@k)DEPS}"; do
        all_nodes[$t]=1
        for dep in ${=DEPS[$t]}; do
            all_nodes[$dep]=1
        done
    done
    for t in "${(@k)all_nodes}"; do
        indeg[$t]=0
    done
    for t in "${(@k)DEPS}"; do
        for dep in ${=DEPS[$t]}; do
            (( indeg[$t]++ ))
        done
    done

    typeset -ga ORDER
    ORDER=()
    typeset -a queue
    queue=()
    for t in "${(@ko)all_nodes}"; do
        if (( indeg[$t] == 0 )); then
            queue+=("$t")
        fi
    done

    local qi=1
    while (( qi <= ${#queue} )); do
        local cur=${queue[qi]}
        (( qi++ ))
        ORDER+=("$cur")
        # Find anyone who depended on cur.
        for t in "${(@ko)DEPS}"; do
            local deps_t="${DEPS[$t]}"
            for dep in ${=deps_t}; do
                if [[ $dep == $cur ]]; then
                    (( indeg[$t]-- ))
                    if (( indeg[$t] == 0 )); then
                        queue+=("$t")
                    fi
                fi
            done
        done
    done

    # If we didn't emit all nodes, there's a cycle.
    if (( ${#ORDER} == ${#all_nodes} )); then
        return 0
    else
        return 1
    fi
}

print_order() {
    if (( ${#ORDER} > 0 )); then
        echo "  order: ${ORDER[*]}"
    else
        echo "  (empty)"
    fi
}

echo "── makefile-like build deps ──"
ts_clear
ts_add main main.o util.o
ts_add main.o main.c util.h
ts_add util.o util.c util.h
ts_add main.c
ts_add util.c
ts_add util.h

if topo_sort; then
    print_order
else
    echo "  cycle!"
fi

echo
echo "── course prerequisites ──"
ts_clear
ts_add "Algorithms" "Data Structures" "Discrete Math"
ts_add "Data Structures" "Programming"
ts_add "OS" "Programming" "Computer Org"
ts_add "Compilers" "Algorithms" "OS"
ts_add "Networks" "Data Structures" "OS"
ts_add "AI" "Algorithms" "Linear Algebra"
ts_add "Discrete Math"
ts_add "Programming"
ts_add "Computer Org"
ts_add "Linear Algebra"
ts_add "Distributed Systems" "Networks" "OS"

if topo_sort; then
    print_order
fi
echo "  total courses: ${#ORDER}"

echo
echo "── cycle detection ──"
ts_clear
ts_add A B
ts_add B C
ts_add C A

if topo_sort; then
    print_order
else
    echo "  ✗ cycle detected (A → B → C → A)"
fi

echo
echo "── partial cycle ──"
ts_clear
ts_add a b
ts_add b c
ts_add c
ts_add x y
ts_add y x   # cycle x ↔ y
ts_add z a

if topo_sort; then
    print_order
else
    echo "  ✗ partial cycle (x ↔ y)"
    echo "  partial order: ${ORDER[*]}"
fi

echo
echo "── linear chain (no parallelism) ──"
ts_clear
ts_add a b
ts_add b c
ts_add c d
ts_add d e
ts_add e

if topo_sort; then
    print_order
fi

echo
echo "── diamond ──"
ts_clear
ts_add d b c
ts_add b a
ts_add c a
ts_add a

if topo_sort; then
    print_order
fi

echo
echo "── apt-style install order ──"
ts_clear
ts_add nginx libpcre openssl
ts_add postgres libssl libxml libreadline
ts_add libssl openssl
ts_add openssl libcrypt
ts_add libxml libxml-base
ts_add libpcre libpcre-base
ts_add libcrypt
ts_add libpcre-base
ts_add libxml-base
ts_add libreadline libncurses
ts_add libncurses

if topo_sort; then
    print_order
    echo "  install order: ${#ORDER} packages"
fi

echo
echo "── stats ──"
echo "  Kahn's algorithm: O(V + E)"
echo "  alternative: Tarjan DFS (3-color)"
echo "  applications: build systems, package mgmt, course planning,"
echo "                spreadsheet evaluation, instruction scheduling"

# === ztest assertions ===
ts_clear
ts_add a b
ts_add b c
ts_add c d
ts_add d e
ts_add e
zassert_ok 1 "linear chain sorted ok" && topo_sort
zassert_eq "${ORDER[*]}" "e d c b a" "linear chain order"
# diamond
ts_clear
ts_add d b c
ts_add b a
ts_add c a
ts_add a
topo_sort
zassert_eq "${ORDER[*]}" "a b c d" "diamond order"
zassert_eq "${ORDER[1]}" "a" "diamond first = a"
zassert_eq "${ORDER[4]}" "d" "diamond last = d"
# cycle detection
ts_clear
ts_add A B
ts_add B C
ts_add C A
if topo_sort; then
    zassert_ok 0 "should detect cycle"
else
    zassert_ok 1 "cycle detected"
fi
# install order count
ts_clear
ts_add nginx libpcre openssl
ts_add postgres libssl libxml libreadline
ts_add libssl openssl
ts_add openssl libcrypt
ts_add libxml libxml-base
ts_add libpcre libpcre-base
ts_add libcrypt
ts_add libpcre-base
ts_add libxml-base
ts_add libreadline libncurses
ts_add libncurses
topo_sort
zassert_eq "${#ORDER}" 11 "install pkg count"
ztest_run
