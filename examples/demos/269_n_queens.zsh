#!/usr/bin/env zshrs
# N-Queens — count solutions + render boards.

typeset -a queens
typeset -a saved1 saved2
typeset -i solutions
N=0

is_safe() {
    local r=$1 c=$2 pr pc
    for ((pr=1; pr<r; pr++)); do
        pc=${queens[pr]}
        (( pc == c )) && return 1
        (( pr - pc == r - c )) && return 1
        (( pr + pc == r + c )) && return 1
    done
    return 0
}

solve() {
    local r=$1 c
    if (( r > N )); then
        (( solutions++ ))
        if (( solutions == 1 )); then
            saved1=( "${queens[@]}" )
        elif (( solutions == 2 )); then
            saved2=( "${queens[@]}" )
        fi
        return
    fi
    for ((c=1; c<=N; c++)); do
        if is_safe $r $c; then
            queens[r]=$c
            solve $((r + 1))
        fi
    done
}

print_board() {
    local -a board
    board=("$@")
    local r c
    for ((r=1; r<=N; r++)); do
        local line=""
        local v=${board[r]}
        for ((c=1; c<=N; c++)); do
            if (( c == v )); then
                line+="♛ "
            else
                line+=". "
            fi
        done
        echo "  $line"
    done
}

for n in 4 5 6; do
    N=$n
    solutions=0
    queens=()
    saved1=()
    saved2=()
    solve 1
    echo "── $n-queens: $solutions solutions ──"
    if (( solutions > 0 )); then
        echo "  first solution: queens at cols (${saved1[*]})"
        print_board "${saved1[@]}"
        if (( solutions > 1 )); then
            echo "  second solution: queens at cols (${saved2[*]})"
            print_board "${saved2[@]}"
        fi
    fi
    echo
done

echo "── solution counts (vs OEIS A000170) ──"
expected=("1" "0" "0" "2" "10" "4" "40")
echo "  expected: 1, 0, 0, 2, 10, 4 for n=1..6"
echo "  computed (n=1..6):"
for n in 1 2 3 4 5 6; do
    N=$n
    solutions=0
    queens=()
    saved1=()
    saved2=()
    solve 1
    exp=${expected[n]}
    mark="✓"
    [[ $solutions != $exp ]] && mark="✗"
    printf "    n=%d : %3d (expected %s) %s\n" $n $solutions $exp $mark
done

# === ztest assertions ===
# OEIS A000170 sequence for n=1..6
count_n() {
    N=$1; solutions=0; queens=(); saved1=(); saved2=(); solve 1; echo $solutions
}
zassert_eq "$(count_n 1)"  1  "1-queens has 1 solution"
zassert_eq "$(count_n 2)"  0  "2-queens has 0 solutions"
zassert_eq "$(count_n 3)"  0  "3-queens has 0 solutions"
zassert_eq "$(count_n 4)"  2  "4-queens has 2 solutions"
zassert_eq "$(count_n 5)"  10 "5-queens has 10 solutions"
zassert_eq "$(count_n 6)"  4  "6-queens has 4 solutions (OEIS A000170)"
# is_safe sanity (with fresh queens array)
N=4; queens=(); queens[1]=2; queens[2]=4
if is_safe 3 1; then zassert_ok  1 "(3,1) safe given Q@(1,2),(2,4)"; else zassert_ok 0 "(3,1) safe"; fi
if is_safe 2 2; then zassert_ok  0 "(2,2) safe (should not be — same col as Q@(1,2)? no, col diff)"; else zassert_ok 1 "(2,2) collides via diagonal/col"; fi
ztest_run
