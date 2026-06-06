#!/usr/bin/env zshrs
# Towers of Hanoi — full move sequence + animated state rendering.

typeset -a TOWER_A TOWER_B TOWER_C

# Init: disk sizes 1..N on tower A.
init_towers() {
    local n=$1 i
    TOWER_A=()
    TOWER_B=()
    TOWER_C=()
    for ((i=n; i>=1; i--)); do
        TOWER_A+=($i)
    done
}

# Render: each tower printed bottom-up to same height.
render() {
    local n=$1 i
    # Get array references via name.
    local -a a b c
    a=("${TOWER_A[@]}")
    b=("${TOWER_B[@]}")
    c=("${TOWER_C[@]}")
    local maxw=$(( n * 2 + 1 ))
    for ((i=n; i>=1; i--)); do
        # Print level i of A, B, C.
        for tname in a b c; do
            local -a t
            eval "t=( \"\${$tname[@]}\" )"
            if (( ${#t} >= i )); then
                local d=${t[i]}
                local block=""
                local k
                for ((k=0; k<d; k++)); do block+="█"; done
                local pad=$(( (maxw - d*2 + 1) / 2 ))
                local sp=""
                for ((k=0; k<pad; k++)); do sp+=" "; done
                printf "%s%s%s  " "$sp" "$block$block" "$sp"
            else
                local pad=$(( maxw / 2 ))
                local sp=""
                local k
                for ((k=0; k<pad; k++)); do sp+=" "; done
                printf "%s│%s  " "$sp" "$sp"
            fi
        done
        echo
    done
    local bar=""
    for ((i=0; i<maxw; i++)); do bar+="─"; done
    printf "%s  %s  %s\n" "$bar" "$bar" "$bar"
    printf " %*s  %*s  %*s\n" $((maxw / 2 + 1)) "A" $((maxw / 2 + 1)) "B" $((maxw / 2 + 1)) "C"
}

# Move top of FROM to TO.
move_disk() {
    local from=$1 to=$2
    local fname=TOWER_$from
    local tname=TOWER_$to
    local -a fa ta
    eval "fa=( \"\${${fname}[@]}\" )"
    eval "ta=( \"\${${tname}[@]}\" )"
    local d=${fa[-1]}
    fa=("${fa[@]:0:-1}")
    ta+=($d)
    case $from in
        A) TOWER_A=("${fa[@]}") ;;
        B) TOWER_B=("${fa[@]}") ;;
        C) TOWER_C=("${fa[@]}") ;;
    esac
    case $to in
        A) TOWER_A=("${ta[@]}") ;;
        B) TOWER_B=("${ta[@]}") ;;
        C) TOWER_C=("${ta[@]}") ;;
    esac
    echo "  move disk $d: $from → $to"
}

move_count=0

# Recursive solver.
hanoi() {
    local n=$1 src=$2 aux=$3 dst=$4
    if (( n == 1 )); then
        move_disk $src $dst
        (( move_count++ ))
        return
    fi
    hanoi $((n-1)) $src $dst $aux
    move_disk $src $dst
    (( move_count++ ))
    hanoi $((n-1)) $aux $src $dst
}

for N in 3 4; do
    echo "═══ Hanoi N=$N (expected 2^N - 1 = $(( 2**N - 1 )) moves) ═══"
    init_towers $N
    echo "initial:"
    render $N
    move_count=0
    hanoi $N A B C
    echo
    echo "final:"
    render $N
    echo "total moves: $move_count"
    echo
done

echo "── move count formula verification ──"
for n in 1 2 3 4 5 6 7 8 10; do
    init_towers $n
    move_count=0
    hanoi $n A B C > /dev/null
    expected=$(( 2**n - 1 ))
    mark="✓"
    [[ $move_count != $expected ]] && mark="✗"
    printf "  n=%2d : %5d moves (expected %5d) %s\n" $n $move_count $expected $mark
done

# === ztest assertions ===
count_moves() {
    init_towers $1; move_count=0; hanoi $1 A B C > /dev/null; echo $move_count
}
zassert_eq "$(count_moves 1)"  1    "n=1: 2^1-1"
zassert_eq "$(count_moves 2)"  3    "n=2: 2^2-1"
zassert_eq "$(count_moves 3)"  7    "n=3: 2^3-1"
zassert_eq "$(count_moves 5)"  31   "n=5: 2^5-1"
zassert_eq "$(count_moves 7)"  127  "n=7: 2^7-1"
zassert_eq "$(count_moves 10)" 1023 "n=10: 2^10-1"
# After solving N=3, all 3 disks should be on C in order [3,2,1] bottom-up
init_towers 3; move_count=0; hanoi 3 A B C > /dev/null
zassert_eq "${#TOWER_A}" 0 "tower A empty after hanoi(3)"
zassert_eq "${#TOWER_B}" 0 "tower B empty after hanoi(3)"
zassert_eq "${#TOWER_C}" 3 "tower C holds 3 disks after hanoi(3)"
zassert_eq "${TOWER_C[1]}" 3 "largest disk at bottom of C"
zassert_eq "${TOWER_C[3]}" 1 "smallest disk at top of C"
ztest_run
