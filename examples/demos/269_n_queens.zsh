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
echo "  expected: 1, 0, 0, 2, 10, 4, 40 for n=1..7"
echo "  computed (n=1..7):"
for n in 1 2 3 4 5 6 7; do
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
