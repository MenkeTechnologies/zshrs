#!/usr/bin/env zshrs
# 15-puzzle — slide tiles + inversion-count solvability check.

typeset -a BOARD
SIZE=4

# Solved state.
init_solved() {
    BOARD=(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 0)
}

print_board() {
    local r c idx v
    echo "  +----+----+----+----+"
    for ((r=0; r<SIZE; r++)); do
        printf "  "
        for ((c=0; c<SIZE; c++)); do
            idx=$(( r*SIZE + c + 1 ))
            v=${BOARD[idx]}
            if (( v == 0 )); then
                printf "|    "
            else
                printf "|%3d " $v
            fi
        done
        echo "|"
        echo "  +----+----+----+----+"
    done
}

# Find blank's index (1..16).
blank_pos() {
    local i
    for ((i=1; i<=16; i++)); do
        [[ ${BOARD[i]} == 0 ]] && { echo $i; return; }
    done
}

# Move blank in direction: U D L R. Returns 1 if invalid.
move() {
    local dir=$1
    local b=$(blank_pos)
    local r=$(( (b - 1) / SIZE ))
    local c=$(( (b - 1) % SIZE ))
    local nr=$r nc=$c
    case $dir in
        U) (( nr-- )) ;;
        D) (( nr++ )) ;;
        L) (( nc-- )) ;;
        R) (( nc++ )) ;;
    esac
    (( nr < 0 || nr >= SIZE || nc < 0 || nc >= SIZE )) && return 1
    local nb=$(( nr*SIZE + nc + 1 ))
    BOARD[b]=${BOARD[nb]}
    BOARD[nb]=0
    return 0
}

# Inversion count (excludes blank).
count_inversions() {
    local inv=0 i j
    for ((i=1; i<=15; i++)); do
        [[ ${BOARD[i]} == 0 ]] && continue
        for ((j=i+1; j<=16; j++)); do
            [[ ${BOARD[j]} == 0 ]] && continue
            (( BOARD[i] > BOARD[j] )) && (( inv++ ))
        done
    done
    echo $inv
}

# Solvability: for 4x4, solvable iff (inversions + blank_row_from_bottom) is odd.
is_solvable() {
    local inv=$(count_inversions)
    local b=$(blank_pos)
    local r=$(( (b - 1) / SIZE ))
    local from_bottom=$(( SIZE - r ))
    local sum=$(( inv + from_bottom ))
    (( sum % 2 == 1 ))
}

echo "── solved state ──"
init_solved
print_board
echo "  inversions: $(count_inversions)   solvable: $(is_solvable && echo YES || echo NO)"

echo
echo "── shuffle via 30 random moves (always solvable) ──"
RANDOM=42
init_solved
dirs=(U D L R)
moves_log=""
for ((i=0; i<30; i++)); do
    d=${dirs[$(( RANDOM % 4 + 1 ))]}
    if move $d 2>/dev/null; then
        moves_log+=$d
    fi
done
echo "  moves: $moves_log"
print_board
echo "  inversions: $(count_inversions)   solvable: $(is_solvable && echo YES || echo NO)"

echo
echo "── reverse the moves (should reach solved) ──"
# Reverse: U↔D, L↔R.
declare -A inv_map=(U D D U L R R L)
for ((i=${#moves_log}; i>=1; i--)); do
    d=${moves_log[i]}
    rev=${inv_map[$d]}
    move $rev > /dev/null
done
print_board
expected_solved=1
for ((i=1; i<=15; i++)); do
    if (( BOARD[i] != i )); then expected_solved=0; break; fi
done
echo "  is solved: $((expected_solved)) (inversions: $(count_inversions))"

echo
echo "── manually-constructed unsolvable ──"
BOARD=(1 2 3 4 5 6 7 8 9 10 11 12 13 15 14 0)   # swap 14 ↔ 15
print_board
inv=$(count_inversions)
solv=$(is_solvable && echo YES || echo NO)
echo "  inversions: $inv   solvable: $solv (expected NO — single transposition)"

# === ztest assertions ===
zassert_eq "$SIZE"     4    "4x4 board"
zassert_eq "${#BOARD}" 16   "16 cells"
zassert_eq "$expected_solved" 1 "reversing scramble restores solved state"
# Test unsolvable detection on current BOARD (14<->15 swap)
zassert_eq "$inv"  1   "single transposition = 1 inversion"
zassert_eq "$solv" "NO" "single-transposition state is unsolvable"
# Restart fresh: solved state has 0 inversions and IS solvable
init_solved
zassert_eq "$(count_inversions)" 0 "solved state has 0 inversions"
if is_solvable; then zassert_ok 1 "solved state is solvable"; else zassert_ok 0 "solved state should be solvable"; fi
zassert_eq "$(blank_pos)" 16 "blank starts at cell 16 in solved layout"
# Single move sanity
move R
if (( $? == 0 )); then zassert_ok 0 "move R from corner should fail"; else zassert_ok 1 "move R off right edge is rejected"; fi
ztest_run
