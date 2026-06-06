#!/usr/bin/env zshrs
# Conway's Game of Life — animated multi-generation, no fn calls in hot loop.

ROWS=8
COLS=18

typeset -a GRID NEXT

init_grid() {
    GRID=()
    local i
    for ((i=1; i<=ROWS*COLS; i++)); do GRID[i]=0; done
}

set_cell() {
    local r=$1 c=$2 v=$3
    local idx=$(( (r-1)*COLS + c ))
    GRID[idx]=$v
}

seed_glider() {
    local r=$1 c=$2
    set_cell $r $((c+1)) 1
    set_cell $((r+1)) $((c+2)) 1
    set_cell $((r+2)) $c 1
    set_cell $((r+2)) $((c+1)) 1
    set_cell $((r+2)) $((c+2)) 1
}

seed_blinker() {
    local r=$1 c=$2
    set_cell $r $c 1
    set_cell $r $((c+1)) 1
    set_cell $r $((c+2)) 1
}

seed_block() {
    local r=$1 c=$2
    set_cell $r $c 1
    set_cell $r $((c+1)) 1
    set_cell $((r+1)) $c 1
    set_cell $((r+1)) $((c+1)) 1
}

# Inline neighbor count + step in single pass.
step() {
    NEXT=()
    local i
    for ((i=1; i<=ROWS*COLS; i++)); do NEXT[i]=0; done
    local r c idx live n
    local nr nc nidx
    for ((r=1; r<=ROWS; r++)); do
        for ((c=1; c<=COLS; c++)); do
            idx=$(( (r-1)*COLS + c ))
            live=${GRID[idx]}
            n=0
            # 8 neighbors inline.
            for nr in $((r-1)) $r $((r+1)); do
                if (( nr >= 1 && nr <= ROWS )); then
                    for nc in $((c-1)) $c $((c+1)); do
                        if (( nc == c && nr == r )); then continue; fi
                        if (( nc >= 1 && nc <= COLS )); then
                            nidx=$(( (nr-1)*COLS + nc ))
                            (( n += GRID[nidx] ))
                        fi
                    done
                fi
            done
            if (( live )); then
                if (( n == 2 || n == 3 )); then NEXT[idx]=1; fi
            else
                if (( n == 3 )); then NEXT[idx]=1; fi
            fi
        done
    done
    GRID=("${NEXT[@]}")
}

print_grid() {
    local title=$1 r c idx
    echo "── $title ──"
    for ((r=1; r<=ROWS; r++)); do
        local line="  "
        for ((c=1; c<=COLS; c++)); do
            idx=$(( (r-1)*COLS + c ))
            if (( GRID[idx] )); then
                line+="█"
            else
                line+="·"
            fi
        done
        echo "$line"
    done
}

count_live() {
    local n=0 i
    for ((i=1; i<=ROWS*COLS; i++)); do
        (( n += GRID[i] ))
    done
    echo $n
}

echo "═══ Conway's Game of Life ═══"

echo
echo "─── Glider (period 4 translation) ───"
init_grid
seed_glider 2 5
print_grid "Gen 0"
step
print_grid "Gen 1"
step
print_grid "Gen 2"
step
step
print_grid "Gen 4 (translated)"

echo
echo "─── Blinker (period 2 oscillator) ───"
init_grid
seed_blinker 5 10
print_grid "Gen 0"
step
print_grid "Gen 1"
step
print_grid "Gen 2 (back to seed)"

echo
echo "─── Block (still life) ───"
init_grid
seed_block 4 8
print_grid "Gen 0"
step
print_grid "Gen 1 (unchanged)"
step
print_grid "Gen 2 (unchanged)"

echo
echo "── statistics ──"
echo "  grid: ${ROWS}x${COLS} = $(( ROWS * COLS )) cells"
echo "  rules: B3/S23 (Conway)"
echo "  seeded patterns: glider, blinker, block"

# === ztest assertions ===
# Block (still life) is invariant
init_grid
seed_block 4 8
b0=$(count_live)
step
b1=$(count_live)
step
b2=$(count_live)
zassert_eq "$b0" "4"  "block: 4 live cells initial"
zassert_eq "$b1" "4"  "block: still life gen 1"
zassert_eq "$b2" "4"  "block: still life gen 2"
# Blinker (period 2)
init_grid
seed_blinker 5 10
bl0=$(count_live)
step
bl1=$(count_live)
step
bl2=$(count_live)
zassert_eq "$bl0" "3"  "blinker: 3 cells initial"
zassert_eq "$bl1" "3"  "blinker: still 3 cells gen 1 (rotated)"
zassert_eq "$bl2" "3"  "blinker: back to gen 0 cell count"
# Glider (5 cells, conserved over period 4)
init_grid
seed_glider 2 5
g0=$(count_live)
step; step; step; step
g4=$(count_live)
zassert_eq "$g0" "5"  "glider: 5 live cells"
zassert_eq "$g4" "5"  "glider: still 5 after period 4"
ztest_run
