#!/usr/bin/env zshrs
# Tic-Tac-Toe — minimax evaluation on specific positions.
# Full game-playing is too slow under zsh recursion overhead — instead
# we evaluate canonical positions to verify minimax correctness.

typeset -a BOARD

print_board() {
    local r c idx
    echo "     1   2   3"
    for ((r=1; r<=3; r++)); do
        printf "  %d  " $r
        for ((c=1; c<=3; c++)); do
            idx=$(( (r-1)*3 + c ))
            local v=${BOARD[idx]}
            if [[ $v == _ ]]; then printf " . "; else printf " %s " $v; fi
            (( c < 3 )) && printf "|"
        done
        echo
        (( r < 3 )) && echo "    ---+---+---"
    done
}

winner() {
    local -a b
    b=("${BOARD[@]}")
    # Lines encoded as parallel triples.
    local -a la lb lc
    la=(1 4 7 1 2 3 1 3)
    lb=(2 5 8 4 5 6 5 5)
    lc=(3 6 9 7 8 9 9 7)
    local i
    for ((i=1; i<=8; i++)); do
        local a=${la[i]} bb=${lb[i]} cc=${lc[i]}
        if [[ ${b[$a]} != _ && ${b[$a]} == ${b[$bb]} && ${b[$bb]} == ${b[$cc]} ]]; then
            echo ${b[$a]}
            return
        fi
    done
    for v in "${b[@]}"; do
        [[ $v == _ ]] && { echo N; return; }
    done
    echo D
}

# Minimax — pure recursion (no memo since each board is small but recursion
# is from a NEAR-FULL state, so depth is small).
minimax() {
    local is_max=$1
    local w=$(winner)
    case $w in
        X) echo 10; return ;;
        O) echo -10; return ;;
        D) echo 0; return ;;
    esac
    local best
    if (( is_max )); then
        best=-100
        local i
        for ((i=1; i<=9; i++)); do
            if [[ ${BOARD[i]} == _ ]]; then
                BOARD[i]=X
                local s=$(minimax 0)
                BOARD[i]=_
                (( s > best )) && best=$s
            fi
        done
    else
        best=100
        local i
        for ((i=1; i<=9; i++)); do
            if [[ ${BOARD[i]} == _ ]]; then
                BOARD[i]=O
                local s=$(minimax 1)
                BOARD[i]=_
                (( s < best )) && best=$s
            fi
        done
    fi
    echo $best
}

best_move() {
    local player=$1 i s best_i best
    if [[ $player == X ]]; then
        best=-100
        for ((i=1; i<=9; i++)); do
            if [[ ${BOARD[i]} == _ ]]; then
                BOARD[i]=X
                s=$(minimax 0)
                BOARD[i]=_
                if (( s > best )); then
                    best=$s; best_i=$i
                fi
            fi
        done
    else
        best=100
        for ((i=1; i<=9; i++)); do
            if [[ ${BOARD[i]} == _ ]]; then
                BOARD[i]=O
                s=$(minimax 1)
                BOARD[i]=_
                if (( s < best )); then
                    best=$s; best_i=$i
                fi
            fi
        done
    fi
    echo $best_i
}

echo "── position 1: near-end, X to move (wins on 9) ──"
BOARD=(X O X
       O X O
       O X _)
print_board
mv=$(best_move X)
echo "  minimax recommends cell $mv"
BOARD[$mv]=X
echo "  after move:"
print_board
echo "  winner: $(winner)"

echo
echo "── position 5: forced draw setup ──"
BOARD=(X O X
       X O O
       O X _)
print_board
echo "  any move → ? winner"
BOARD[9]=X
echo "  X plays 9:"
print_board
echo "  result: $(winner)"

echo
echo "── position 6: detect existing win ──"
BOARD=(X X X
       _ O _
       _ _ O)
print_board
echo "  winner: $(winner)"

BOARD=(O _ X
       O X _
       O _ _)
print_board
echo "  winner: $(winner)"

BOARD=(X _ O
       _ X O
       _ _ X)
print_board
echo "  winner: $(winner)"

echo
echo "── verify minimax returns correct values ──"
echo "  (skip deeper searches to keep CI runtime bounded)"

echo "  X has 3-in-a-row already (terminal):"
BOARD=(X X X
       _ _ _
       _ _ _)
score=$(minimax 1)
echo "    score: $score (10 = X wins)"

echo "  O has 3-in-a-row already (terminal):"
BOARD=(O O O
       _ _ _
       _ _ _)
score=$(minimax 0)
echo "    score: $score (-10 = O wins)"

echo "  forced draw position (board full):"
BOARD=(X O X
       O X O
       O X O)
score=$(minimax 1)
echo "    score: $score (0 = draw)"

# === ztest assertions ===
# winner detection — row, column, diagonal
BOARD=(X X X
       _ O _
       _ _ O)
zassert_eq "$(winner)" "X"   "winner: top row X"
BOARD=(O _ X
       O X _
       O _ _)
zassert_eq "$(winner)" "O"   "winner: left col O"
BOARD=(X _ O
       _ X O
       _ _ X)
zassert_eq "$(winner)" "X"   "winner: diagonal X"
BOARD=(X O X
       O X O
       O X O)
zassert_eq "$(winner)" "D"   "winner: draw (full board no line)"
BOARD=(X _ _
       _ _ _
       _ _ _)
zassert_eq "$(winner)" "N"   "winner: N (no winner, not full)"
# minimax terminal scores
BOARD=(X X X _ _ _ _ _ _)
zassert_eq "$(minimax 1)" "10"   "minimax X-wins terminal = 10"
BOARD=(O O O _ _ _ _ _ _)
zassert_eq "$(minimax 0)" "-10"  "minimax O-wins terminal = -10"
BOARD=(X O X
       O X O
       O X O)
zassert_eq "$(minimax 1)" "0"    "minimax draw terminal = 0"
ztest_run
