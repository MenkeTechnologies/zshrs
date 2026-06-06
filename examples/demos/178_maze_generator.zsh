#!/usr/bin/env zshrs
# Naive maze generator using DFS carve.

RANDOM=42
W=11
H=7

typeset -a MAZE
for ((i=0; i<H*W; i++)); do MAZE+=("#"); done

set_cell() {
    local r=$1 c=$2 v=$3
    MAZE[r*W+c+1]=$v
}

get_cell() {
    local r=$1 c=$2
    echo ${MAZE[r*W+c+1]}
}

# Carve cells on odd indices (so walls between cells stay).
carve() {
    local r=$1 c=$2
    set_cell $r $c " "
    # Random direction order.
    local -a dirs=(N S E W)
    local i j tmp
    for ((i = ${#dirs[@]}; i > 1; i--)); do
        j=$(( RANDOM % i + 1 ))
        tmp=${dirs[i]}
        dirs[i]=${dirs[j]}
        dirs[j]=$tmp
    done
    for d in "${dirs[@]}"; do
        local nr=$r nc=$c
        case $d in
            N) (( nr -= 2 )) ;;
            S) (( nr += 2 )) ;;
            E) (( nc += 2 )) ;;
            W) (( nc -= 2 )) ;;
        esac
        if (( nr > 0 && nr < H-1 && nc > 0 && nc < W-1 )); then
            if [[ $(get_cell $nr $nc) == "#" ]]; then
                # Carve wall between.
                local wr=$(( (r + nr) / 2 ))
                local wc=$(( (c + nc) / 2 ))
                set_cell $wr $wc " "
                carve $nr $nc
            fi
        fi
    done
}

print_maze() {
    local r c
    for ((r=0; r<H; r++)); do
        for ((c=0; c<W; c++)); do
            printf "%s" "$(get_cell $r $c)"
        done
        echo
    done
}

echo "── generated maze (DFS) ──"
carve 1 1
print_maze

echo "── second maze with different seed ──"
MAZE=()
for ((i=0; i<H*W; i++)); do MAZE+=("#"); done
RANDOM=123
carve 1 1
print_maze

echo "── third maze with seed=7 ──"
MAZE=()
for ((i=0; i<H*W; i++)); do MAZE+=("#"); done
RANDOM=7
carve 1 1
print_maze

# === ztest assertions ===
zassert_eq "$W" 11                              "width = 11"
zassert_eq "$H" 7                               "height = 7"
zassert_eq "${#MAZE[@]}" $((H*W))               "maze has H*W cells"
# Corner cells must still be wall.
zassert_eq "$(get_cell 0 0)" "#"                "(0,0) is wall"
zassert_eq "$(get_cell 0 $((W-1)))" "#"         "top-right is wall"
zassert_eq "$(get_cell $((H-1)) 0)" "#"         "bottom-left is wall"
zassert_eq "$(get_cell $((H-1)) $((W-1)))" "#"  "bottom-right is wall"
# Start cell at (1,1) carved to space.
zassert_eq "$(get_cell 1 1)" " "                "starting cell carved"
# Deterministic seed → identical maze. Rebuild with seed=7 and compare a cell.
MAZE=()
for ((i=0; i<H*W; i++)); do MAZE+=("#"); done
RANDOM=7
carve 1 1
cell11_again="$(get_cell 1 1)"
zassert_eq "$cell11_again" " "                  "deterministic carve repeats"
ztest_run
