#!/usr/bin/env zshrs
# Sudoku grid validator — check rows, columns, 3x3 blocks.

typeset -a GRID

set_cell() {
    local r=$1 c=$2 v=$3 idx
    idx=$(( (r-1)*9 + c ))
    GRID[idx]=$v
}

get_cell() {
    local r=$1 c=$2 idx
    idx=$(( (r-1)*9 + c ))
    echo "${GRID[idx]}"
}

load_grid() {
    GRID=()
    local row v i
    for row in "$@"; do
        for ((i=1; i<=9; i++)); do
            v=${row[i]}
            [[ $v == . ]] && v=0
            GRID+=($v)
        done
    done
}

print_grid() {
    local r c v idx
    for r in {1..9}; do
        for c in {1..9}; do
            idx=$(( (r-1)*9 + c ))
            v="${GRID[idx]}"
            if [[ $v == 0 ]]; then
                printf ". "
            else
                printf "%d " $v
            fi
            if (( c % 3 == 0 && c < 9 )); then
                printf "| "
            fi
        done
        printf "\n"
        if (( r % 3 == 0 && r < 9 )); then
            printf "%s\n" "------+-------+------"
        fi
    done
}

check_set() {
    local -a seen
    seen=()
    local v s
    for v in "$@"; do
        if (( v == 0 )); then continue; fi
        if (( v < 1 || v > 9 )); then return 1; fi
        for s in "${seen[@]}"; do
            [[ $s == $v ]] && return 1
        done
        seen+=($v)
    done
    return 0
}

validate_rows() {
    local r c idx
    for r in {1..9}; do
        local -a row_vals
        row_vals=()
        for c in {1..9}; do
            idx=$(( (r-1)*9 + c ))
            row_vals+=("${GRID[idx]}")
        done
        if ! check_set "${row_vals[@]}"; then return 1; fi
    done
    return 0
}

validate_cols() {
    local r c idx
    for c in {1..9}; do
        local -a col_vals
        col_vals=()
        for r in {1..9}; do
            idx=$(( (r-1)*9 + c ))
            col_vals+=("${GRID[idx]}")
        done
        if ! check_set "${col_vals[@]}"; then return 1; fi
    done
    return 0
}

validate_blocks() {
    local br bc dr dc idx
    for br in 0 3 6; do
        for bc in 0 3 6; do
            local -a block_vals
            block_vals=()
            for ((dr=1; dr<=3; dr++)); do
                for ((dc=1; dc<=3; dc++)); do
                    idx=$(( (br+dr-1)*9 + bc+dc ))
                    block_vals+=("${GRID[idx]}")
                done
            done
            if ! check_set "${block_vals[@]}"; then return 1; fi
        done
    done
    return 0
}

validate_grid() {
    validate_rows && validate_cols && validate_blocks
}

echo "── valid partial grid ──"
load_grid \
    "53..7...." \
    "6..195..." \
    ".98....6." \
    "8...6...3" \
    "4..8.3..1" \
    "7...2...6" \
    ".6....28." \
    "...419..5" \
    "....8..79"
print_grid
if validate_grid; then echo "VALID"; else echo "INVALID"; fi

echo
echo "── invalid (duplicate 5 in row 1) ──"
load_grid \
    "53.5....." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........."
print_grid
if validate_grid; then echo "VALID"; else echo "INVALID"; fi

echo
echo "── invalid (duplicate in column 1) ──"
load_grid \
    "5........" \
    "........." \
    "........." \
    "5........" \
    "........." \
    "........." \
    "........." \
    "........." \
    "........."
print_grid
if validate_grid; then echo "VALID"; else echo "INVALID"; fi

echo
echo "── invalid (duplicate in 3x3 block) ──"
load_grid \
    "5........" \
    ".5......." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........."
print_grid
if validate_grid; then echo "VALID"; else echo "INVALID"; fi

echo
echo "── empty grid (valid) ──"
load_grid \
    "........." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........." \
    "........."
if validate_grid; then echo "VALID"; else echo "INVALID"; fi

# === ztest assertions ===
load_grid \
    "53..7...." \
    "6..195..." \
    ".98....6." \
    "8...6...3" \
    "4..8.3..1" \
    "7...2...6" \
    ".6....28." \
    "...419..5" \
    "....8..79"
zassert_eq "$(get_cell 1 1)" "5" "cell (1,1) = 5"
zassert_eq "$(get_cell 1 2)" "3" "cell (1,2) = 3"
zassert_eq "$(get_cell 9 9)" "9" "cell (9,9) = 9"
zassert_eq "$(get_cell 1 3)" "0" "cell (1,3) blank → 0"
if validate_grid; then zassert_ok 1  "classic puzzle is valid"; else zassert_ok 0 "classic puzzle is valid"; fi
load_grid \
    "53.5....." \
    "........." "........." "........." "........." "........." \
    "........." "........." "........."
if validate_grid; then zassert_ok 0 "dup-in-row rejected"; else zassert_ok 1 "dup-in-row rejected"; fi
load_grid \
    "5........" "........." "........." "5........" "........." \
    "........." "........." "........." "........."
if validate_grid; then zassert_ok 0 "dup-in-col rejected"; else zassert_ok 1 "dup-in-col rejected"; fi
load_grid \
    "5........" ".5......." "........." "........." "........." \
    "........." "........." "........." "........."
if validate_grid; then zassert_ok 0 "dup-in-block rejected"; else zassert_ok 1 "dup-in-block rejected"; fi
load_grid \
    "........." "........." "........." "........." "........." \
    "........." "........." "........." "........."
if validate_grid; then zassert_ok 1 "empty grid is valid"; else zassert_ok 0 "empty grid is valid"; fi
ztest_run
