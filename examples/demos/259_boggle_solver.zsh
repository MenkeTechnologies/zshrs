#!/usr/bin/env zshrs
# Boggle solver — find dictionary words in 4×4 grid via 8-dir DFS.

# Grid (uppercase).
typeset -a GRID
GRID=(
    "C A T S"
    "E R O P"
    "I N L E"
    "M P I T"
)

ROWS=4
COLS=4

cell_at() {
    local r=$1 c=$2
    if (( r < 1 || r > ROWS || c < 1 || c > COLS )); then echo ""; return; fi
    local row=${GRID[r]}
    local nospc=${row// /}
    echo "${nospc[c]}"
}

# Dictionary (small).
DICT=(
    CAT CATS RAT RATS NET NETS TIN PIN PILE
    PILES PINE PINES TINE LINE LINES MINE
    MINES CONE STONE TONE TENOR SNORE
    SORE TORE CORE PORE LORE NORE
    OIL TOIL COIL FOIL
    EAT EATS REAR NEAR TEAR
    ROPE TOPE OPER POSE PROSE
    LIP TIP NIP PEN PET LET CET
    ROLE POLE TOLE
)

# Check if path exists from (r,c) for word starting at offset i.
dfs() {
    local r=$1 c=$2 word=$3 i=$4 visited=$5
    local len=${#word}
    if (( i > len )); then return 0; fi
    local target=${word[i]}
    local got=$(cell_at $r $c)
    if [[ $got != $target ]]; then return 1; fi
    # Mark visited (r*ROWS + c).
    local key="${r}_${c}"
    if [[ $visited == *,$key,* ]]; then return 1; fi
    local new_visited="${visited}${key},"
    # Try all 8 neighbors.
    local dr dc
    for dr in -1 0 1; do
        for dc in -1 0 1; do
            (( dr == 0 && dc == 0 )) && continue
            if dfs $((r+dr)) $((c+dc)) "$word" $((i+1)) "$new_visited"; then
                return 0
            fi
        done
    done
    return 1
}

find_word_in_grid() {
    local word=$1 r c
    for ((r=1; r<=ROWS; r++)); do
        for ((c=1; c<=COLS; c++)); do
            if dfs $r $c "$word" 1 ","; then
                return 0
            fi
        done
    done
    return 1
}

echo "── grid ──"
local r
for r in $(seq 1 $ROWS); do
    echo "  ${GRID[r]}"
done

echo
echo "── searching dictionary (${#DICT} words) ──"
typeset -a found
for w in "${DICT[@]}"; do
    if find_word_in_grid "$w"; then
        found+=("$w")
    fi
done

echo "  found ${#found} words:"
for w in "${(o)found[@]}"; do
    printf "    %s (len %d)\n" "$w" "${#w}"
done

echo
echo "── stats ──"
echo "  dict size:  ${#DICT}"
echo "  found:      ${#found}"
echo "  hit rate:   $(( ${#found} * 100 / ${#DICT} ))%"

echo
echo "── longest found ──"
longest=""
for w in "${found[@]}"; do
    if (( ${#w} > ${#longest} )); then
        longest=$w
    fi
done
echo "  longest: '$longest' (${#longest} chars)"

# === ztest assertions ===
zassert_eq "$ROWS" "4" "4x4 grid"
zassert_eq "$COLS" "4" "4 cols"
zassert_eq "$(cell_at 1 1)" "C" "cell (1,1) = C"
zassert_eq "$(cell_at 1 4)" "S" "cell (1,4) = S"
zassert_eq "$(cell_at 4 4)" "T" "cell (4,4) = T"
zassert_eq "$(cell_at 5 1)" "" "out-of-bounds row → empty"
zassert_eq "$(cell_at 1 0)" "" "out-of-bounds col → empty"
zassert_eq "${#DICT[@]}" "52" "52 dictionary words"
if find_word_in_grid CAT; then zassert_ok 1 "CAT findable"; else zassert_ok 0 "CAT findable"; fi
if find_word_in_grid STONE; then zassert_ok 1 "STONE findable"; else zassert_ok 0 "STONE findable"; fi
if find_word_in_grid PYTHON; then zassert_ok 0 "PYTHON not in grid"; else zassert_ok 1 "PYTHON not in grid"; fi
zassert_eq "${#found[@]}" "31"  "31 of 52 dict words found in the grid"
zassert_eq "$longest" "STONE"   "longest found = STONE"
ztest_run
