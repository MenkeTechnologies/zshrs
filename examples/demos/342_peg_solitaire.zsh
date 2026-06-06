#!/usr/bin/env zshrs
# Peg solitaire — small board, simulate moves, count remaining pegs.

# Cross-shape board (7x7 with corners cut):
#     X X X
#     X X X
# X X X X X X X
# X X X _ X X X
# X X X X X X X
#     X X X
#     X X X
# Each cell: 'P' peg, '_' hole, 'X' off-board.

typeset -a BOARD
SIZE=7

init_board() {
    BOARD=()
    local r c idx
    for ((r=1; r<=SIZE; r++)); do
        for ((c=1; c<=SIZE; c++)); do
            idx=$(( (r-1)*SIZE + c ))
            # Cross shape.
            if (( (r >= 1 && r <= 2) || (r >= 6 && r <= 7) )); then
                if (( c >= 3 && c <= 5 )); then
                    BOARD[idx]=P
                else
                    BOARD[idx]=X   # off-board
                fi
            else
                BOARD[idx]=P
            fi
        done
    done
    # Center hole.
    local center_idx=$(( 3*SIZE + 4 ))
    BOARD[$center_idx]=_
}

print_board() {
    local r c idx v
    for ((r=1; r<=SIZE; r++)); do
        printf "  "
        for ((c=1; c<=SIZE; c++)); do
            idx=$(( (r-1)*SIZE + c ))
            v=${BOARD[idx]}
            case $v in
                P) printf "● " ;;
                _) printf "○ " ;;
                X) printf "  " ;;
            esac
        done
        echo
    done
}

# Cell at (r,c).
cell_v() {
    local r=$1 c=$2
    if (( r < 1 || r > SIZE || c < 1 || c > SIZE )); then
        echo "X"
        return
    fi
    local idx=$(( (r-1)*SIZE + c ))
    echo ${BOARD[idx]}
}

set_cell_v() {
    local r=$1 c=$2 v=$3
    local idx=$(( (r-1)*SIZE + c ))
    BOARD[idx]=$v
}

# Try a move: jump from (r,c) over (mr,mc) to (tr,tc).
try_move() {
    local r=$1 c=$2 dr=$3 dc=$4
    local mr=$(( r + dr ))
    local mc=$(( c + dc ))
    local tr=$(( r + 2*dr ))
    local tc=$(( c + 2*dc ))
    [[ $(cell_v $r $c) == P ]] || return 1
    [[ $(cell_v $mr $mc) == P ]] || return 1
    [[ $(cell_v $tr $tc) == _ ]] || return 1
    set_cell_v $r $c _
    set_cell_v $mr $mc _
    set_cell_v $tr $tc P
    return 0
}

count_pegs() {
    local n=0 i
    for ((i=1; i<=SIZE*SIZE; i++)); do
        [[ ${BOARD[i]} == P ]] && (( n++ ))
    done
    echo $n
}

# Find first valid move (greedy).
find_any_move() {
    local r c dr dc
    typeset -a DIRS_R DIRS_C
    DIRS_R=(-1 1 0 0)
    DIRS_C=(0 0 -1 1)
    for ((r=1; r<=SIZE; r++)); do
        for ((c=1; c<=SIZE; c++)); do
            if [[ $(cell_v $r $c) == P ]]; then
                local i
                for ((i=1; i<=4; i++)); do
                    dr=${DIRS_R[i]}
                    dc=${DIRS_C[i]}
                    local mr=$(( r + dr ))
                    local mc=$(( c + dc ))
                    local tr=$(( r + 2*dr ))
                    local tc=$(( c + 2*dc ))
                    if [[ $(cell_v $mr $mc) == P ]] && [[ $(cell_v $tr $tc) == _ ]]; then
                        echo "$r $c $dr $dc"
                        return 0
                    fi
                done
            fi
        done
    done
    return 1
}

echo "── initial board ──"
init_board
print_board
echo "  pegs: $(count_pegs)"

echo
echo "── scripted moves ──"
moves=(
    "5 4 -1 0"
    "3 4 -1 0"
    "2 4 1 0"
    "6 4 -1 0"
    "4 4 -1 0"
)
for m in "${moves[@]}"; do
    set -- ${=m}
    if try_move $1 $2 $3 $4; then
        echo "  move ($1,$2) dir=($3,$4) ✓"
    else
        echo "  move ($1,$2) dir=($3,$4) ✗ invalid"
    fi
done
print_board
echo "  pegs: $(count_pegs)"

echo
echo "── greedy solver (find first valid move repeatedly) ──"
init_board
print_board
echo "  initial pegs: $(count_pegs)"

iter=0
while true; do
    move=$(find_any_move)
    [[ -z $move ]] && break
    set -- ${=move}
    try_move $1 $2 $3 $4 > /dev/null
    (( iter++ ))
done

echo
echo "── after $iter greedy moves ──"
print_board
remaining=$(count_pegs)
echo "  remaining pegs: $remaining"
if (( remaining == 1 )); then
    echo "  ✓ solved!"
else
    echo "  ✗ greedy gets stuck (proper solution requires backtracking)"
fi

echo
echo "── peg-count after each phase ──"
init_board
phases=(
    "5 4 -1 0:phase1"
    "3 4 -1 0:phase2"
    "2 4 1 0:phase3"
)
for entry in "${phases[@]}"; do
    move="${entry%:*}"
    name="${entry#*:}"
    set -- ${=move}
    try_move $1 $2 $3 $4 > /dev/null
    echo "  after $name: pegs = $(count_pegs)"
done

echo
echo "── board geometry ──"
echo "  total cells: $(( SIZE * SIZE )) = 49"
echo "  active cells (cross): 33"
echo "  initial pegs: 32"
echo "  target: 1 peg in center"

echo
echo "── notation ──"
echo "  peg layout uses standard 33-cell English board"
echo "  proven: with optimal play, exactly 1 peg can be left in center"
echo "  this requires backtracking (greedy gets stuck around 5-8 pegs)"

# === ztest assertions ===
init_board
zassert_eq "$(count_pegs)" 32 "32 initial pegs"
zassert_eq "$(cell_v 4 4)" "_"  "center is hole"
zassert_eq "$(cell_v 1 1)" "X"  "corner is off-board"
zassert_eq "$(cell_v 4 1)" "P"  "left arm has peg"
# Reset and apply one valid move
init_board
if try_move 2 4 1 0; then zassert_ok 1 "valid jump"
else zassert_ok 0 "jump should be valid"; fi
zassert_eq "$(count_pegs)" 31 "31 after one jump"
# Invalid: can't jump into a peg
if try_move 1 1 1 0; then zassert_ok 0 "invalid jump succeeded"
else zassert_ok 1 "invalid jump rejected"; fi
ztest_run
