#!/usr/bin/env zshrs
# Tic-tac-toe — scripted game with move sequence + win detection.

typeset -a BOARD
init_board() {
    BOARD=("." "." "." "." "." "." "." "." ".")
}

print_board() {
    echo " ${BOARD[1]} | ${BOARD[2]} | ${BOARD[3]}"
    echo "---+---+---"
    echo " ${BOARD[4]} | ${BOARD[5]} | ${BOARD[6]}"
    echo "---+---+---"
    echo " ${BOARD[7]} | ${BOARD[8]} | ${BOARD[9]}"
}

# Cell indexed 1..9: positions 1..3 top row, 4..6 mid, 7..9 bot.

place() {
    local pos=$1 sym=$2
    if [[ ${BOARD[pos]} != "." ]]; then
        echo "  invalid: pos $pos already occupied"
        return 1
    fi
    BOARD[pos]=$sym
}

check_win() {
    local lines=(
        "1 2 3" "4 5 6" "7 8 9"   # rows
        "1 4 7" "2 5 8" "3 6 9"   # columns
        "1 5 9" "3 5 7"           # diagonals
    )
    for line in "${lines[@]}"; do
        set -- $=line
        if [[ ${BOARD[$1]} != "." && ${BOARD[$1]} == ${BOARD[$2]} && ${BOARD[$2]} == ${BOARD[$3]} ]]; then
            echo "${BOARD[$1]}"
            return 0
        fi
    done
    echo "-"
    return 1
}

echo "── game 1: X wins on diagonal ──"
init_board
moves=(1 5 3 7 9)  # X at 1, O at 5, X at 3, O at 7, X at 9 (1-5-9 diag)
turn=X
for m in "${moves[@]}"; do
    place $m $turn
    if [[ $turn == X ]]; then turn=O; else turn=X; fi
done
print_board
result=$(check_win)
[[ $result != "-" ]] && echo "Winner: $result" || echo "no winner yet"

echo
echo "── game 2: scratch (draw) ──"
init_board
moves=(1 2 3 4 6 5 7 9 8)
turn=X
for m in "${moves[@]}"; do
    place $m $turn
    if [[ $turn == X ]]; then turn=O; else turn=X; fi
done
print_board
result=$(check_win)
[[ $result == "-" ]] && echo "Draw"

echo
echo "── game 3: O wins on column ──"
init_board
moves=(1 2 4 5 7 8)  # O wins 2-5-8 column
turn=X
for m in "${moves[@]}"; do
    place $m $turn
    if [[ $turn == X ]]; then turn=O; else turn=X; fi
done
# Wait — X went first: X(1), O(2), X(4), O(5), X(7), O(8). O placed at 2,5,8 — column!
print_board
result=$(check_win)
echo "Winner: $result"
