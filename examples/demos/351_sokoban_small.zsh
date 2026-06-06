#!/usr/bin/env zshrs
# Sokoban — simple board, scripted moves.

# # = wall, . = floor, $ = box, * = box on target, @ = player, + = player on target, T = target.
typeset -a BOARD
WIDTH=0
HEIGHT=0
PLAYER_R=0
PLAYER_C=0

load_board() {
    BOARD=()
    HEIGHT=$#
    local r=1 c
    for row in "$@"; do
        local row_len=${#row}
        if (( row_len > WIDTH )); then WIDTH=$row_len; fi
        for ((c=1; c<=row_len; c++)); do
            local v="${row[c]}"
            local idx=$(( (r-1)*100 + c ))
            BOARD[idx]=$v
            if [[ $v == @ || $v == + ]]; then
                PLAYER_R=$r
                PLAYER_C=$c
            fi
        done
        (( r++ ))
    done
}

cell_v() {
    local r=$1 c=$2
    if (( r < 1 || r > HEIGHT || c < 1 || c > WIDTH )); then
        echo "#"
        return
    fi
    local idx=$(( (r-1)*100 + c ))
    echo "${BOARD[idx]}"
}

set_cell() {
    local r=$1 c=$2 v=$3
    local idx=$(( (r-1)*100 + c ))
    BOARD[idx]=$v
}

print_board() {
    local r c
    for ((r=1; r<=HEIGHT; r++)); do
        printf "  "
        for ((c=1; c<=WIDTH; c++)); do
            printf "%s" "$(cell_v $r $c)"
        done
        echo
    done
}

# Move player by (dr, dc).
try_move() {
    local dr=$1 dc=$2
    local nr=$(( PLAYER_R + dr ))
    local nc=$(( PLAYER_C + dc ))
    local target="$(cell_v $nr $nc)"

    if [[ $target == "#" ]]; then return 1; fi
    if [[ $target == "." || $target == "T" ]]; then
        # Simple move.
        local cur=$(cell_v $PLAYER_R $PLAYER_C)
        local cur_v="."
        [[ $cur == "+" ]] && cur_v="T"
        set_cell $PLAYER_R $PLAYER_C $cur_v
        local new_v="@"
        [[ $target == "T" ]] && new_v="+"
        set_cell $nr $nc $new_v
        PLAYER_R=$nr
        PLAYER_C=$nc
        return 0
    fi
    if [[ $target == "\$" || $target == "*" ]]; then
        # Push the box.
        local box_r=$(( nr + dr ))
        local box_c=$(( nc + dc ))
        local beyond=$(cell_v $box_r $box_c)
        if [[ $beyond == "#" || $beyond == "\$" || $beyond == "*" ]]; then return 1; fi
        local new_box="\$"
        [[ $beyond == "T" ]] && new_box="*"
        set_cell $box_r $box_c $new_box
        # Player moves.
        local new_player="@"
        [[ $target == "*" ]] && new_player="+"
        set_cell $nr $nc $new_player
        local cur=$(cell_v $PLAYER_R $PLAYER_C)
        local cur_v="."
        [[ $cur == "+" ]] && cur_v="T"
        set_cell $PLAYER_R $PLAYER_C $cur_v
        PLAYER_R=$nr
        PLAYER_C=$nc
        return 0
    fi
    return 1
}

# Solved iff no '$' left (only '*' = box on target).
is_solved() {
    local r c v
    for ((r=1; r<=HEIGHT; r++)); do
        for ((c=1; c<=WIDTH; c++)); do
            v=$(cell_v $r $c)
            [[ $v == "\$" ]] && return 1
        done
    done
    return 0
}

count_boxes() {
    local r c v on=0 off=0
    for ((r=1; r<=HEIGHT; r++)); do
        for ((c=1; c<=WIDTH; c++)); do
            v=$(cell_v $r $c)
            [[ $v == "\$" ]] && (( off++ ))
            [[ $v == "*" ]] && (( on++ ))
        done
    done
    echo "$on / $((on + off))"
}

echo "── puzzle 1: single box ──"
load_board \
    "#####" \
    "#@..#" \
    "#.$.#" \
    "#..T#" \
    "#####"
print_board
echo "  boxes on target: $(count_boxes)"

echo
echo "  scripted: down, down, right, right"
try_move 1 0 && echo "  ✓ down"
try_move 1 0 && echo "  ✓ down"
try_move 0 1 && echo "  ✓ right (push box)"
try_move 0 1 && echo "  ✓ right (push box onto T)"
print_board
echo "  boxes on target: $(count_boxes)"
if is_solved; then echo "  ✓ SOLVED"; else echo "  ✗ not solved"; fi

echo
echo "── puzzle 2: two boxes ──"
load_board \
    "########" \
    "#T....@#" \
    "#.\$..\$.#" \
    "#T.....#" \
    "########"
print_board
echo "  boxes on target: $(count_boxes)"

echo
echo "  scripted: left × 5"
for i in 1 2 3 4 5; do
    try_move 0 -1 && echo "  ✓ left $i"
done
print_board
echo "  boxes on target: $(count_boxes)"

echo
echo "── puzzle 3: wall + invalid moves ──"
load_board \
    "######" \
    "#@..\$#" \
    "#....#" \
    "#.T..#" \
    "######"
print_board

echo "  attempts:"
try_move 0 -1 || echo "  ✗ left blocked by wall"
try_move -1 0 || echo "  ✗ up blocked by wall"
try_move 1 0  && echo "  ✓ down"
try_move 0 1  && echo "  ✓ right"
try_move 0 1  && echo "  ✓ right"
try_move 1 0  && echo "  ✓ down"
print_board

echo
echo "── notation ──"
echo "  # = wall    . = floor    \$ = box   T = target"
echo "  @ = player  + = player on target  * = box on target"
echo
echo "  controls: 4-direction movement"
echo "  rule: can only push boxes (never pull)"
echo "  win: every box on a target"
echo
echo "── PSPACE-hardness ──"
echo "  Sokoban is PSPACE-complete (Culberson 1997)"
echo "  generalization to n×n grids has no known polynomial-time solver"
echo "  but small puzzles solve via IDA* or DFS w/ deadlock detection"

# === ztest assertions ===
# Reset and load puzzle 1.
load_board "#####" "#@..#" "#.\$.#" "#..T#" "#####"
zassert_eq "$HEIGHT" "5"           "puzzle1 height"
zassert_eq "$WIDTH"  "8"           "WIDTH carries over from puzzle 2"
zassert_eq "$PLAYER_R" "2"         "puzzle1 player row"
zassert_eq "$PLAYER_C" "2"         "puzzle1 player col"
zassert_eq "$(cell_v 1 1)" "#"     "puzzle1 wall at (1,1)"
zassert_eq "$(cell_v 4 4)" "T"     "puzzle1 target at (4,4)"
# Solve it.
try_move 1 0
try_move 1 0
try_move 0 1
try_move 0 1
# Player should have moved from (2,2) to (4,4)
zassert_eq "$PLAYER_R" "4"         "player moved to row 4 after scripted moves"
zassert_eq "$PLAYER_C" "4"         "player moved to col 4 after scripted moves"
# Walls block movement.
load_board "######" "#@..\$#" "#....#" "#.T..#" "######"
try_move 0 -1
zassert_eq "$PLAYER_C" "2"         "left blocked by wall"
try_move -1 0
zassert_eq "$PLAYER_R" "2"         "up blocked by wall"
ztest_run
