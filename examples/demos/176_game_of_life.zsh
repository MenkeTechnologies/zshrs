#!/usr/bin/env zshrs
# Conway's Game of Life — 5 generations on a small grid.

ROWS=8
COLS=20

typeset -a GRID NEXT
init_grid() {
    GRID=()
    local r c
    for ((r=0; r<ROWS; r++)); do
        for ((c=0; c<COLS; c++)); do
            GRID+=(0)
        done
    done
}

get_cell() {
    local r=$1 c=$2
    if (( r < 0 || r >= ROWS || c < 0 || c >= COLS )); then echo 0; return; fi
    echo ${GRID[r*COLS+c+1]}
}

set_cell() {
    local r=$1 c=$2 v=$3
    GRID[r*COLS+c+1]=$v
}

print_grid() {
    local r c
    for ((r=0; r<ROWS; r++)); do
        for ((c=0; c<COLS; c++)); do
            if (( ${GRID[r*COLS+c+1]} )); then
                printf "█"
            else
                printf "·"
            fi
        done
        echo
    done
}

step() {
    NEXT=()
    local r c
    for ((r=0; r<ROWS; r++)); do
        for ((c=0; c<COLS; c++)); do
            local n=0
            local dr dc
            for dr in -1 0 1; do
                for dc in -1 0 1; do
                    if (( dr == 0 && dc == 0 )); then continue; fi
                    (( n += $(get_cell $((r+dr)) $((c+dc))) ))
                done
            done
            local alive=$(get_cell $r $c)
            local new=0
            if (( alive )); then
                (( n == 2 || n == 3 )) && new=1
            else
                (( n == 3 )) && new=1
            fi
            NEXT+=($new)
        done
    done
    GRID=("${NEXT[@]}")
}

init_grid

# Glider pattern at (1,2).
set_cell 1 2 1
set_cell 2 3 1
set_cell 3 1 1
set_cell 3 2 1
set_cell 3 3 1

# Add a blinker.
set_cell 5 10 1
set_cell 5 11 1
set_cell 5 12 1

echo "── generation 0 ──"
print_grid

for gen in 1 2 3 4; do
    step
    echo "── generation $gen ──"
    print_grid
done

echo "── final cell count ──"
alive=0
for v in "${GRID[@]}"; do (( alive += v )); done
echo "alive: $alive"

# === ztest assertions ===
zassert_eq "$ROWS" 8                   "8 rows"
zassert_eq "$COLS" 20                  "20 cols"
zassert_eq "${#GRID[@]}" $((8*20))     "grid is 160 cells"
# After 4 generations, final alive count is 8.
zassert_eq "$alive" 8                  "8 cells alive after 4 generations"
# get_cell stays in bounds.
zassert_eq "$(get_cell -1 0)" 0       "off-grid (-1) returns 0"
zassert_eq "$(get_cell 0 100)" 0      "off-grid (col=100) returns 0"
# Each cell is 0 or 1.
sample=${GRID[1]}
zassert_match '^[01]$' "$sample"      "cell value is 0/1"
ztest_run
