#!/usr/bin/env zshrs
# Lights Out puzzle — 5x5 grid; pressing toggles cell + neighbors.
# Note: avoids inner-loop cmd-substitution which has flaky semantics
# in zshrs (see docs/BUGS.md #10) — BOARD accessed directly via
# precomputed index.

typeset -a BOARD
SIZE=5

init_board() {
    BOARD=()
    local i
    for ((i=0; i<SIZE*SIZE; i++)); do BOARD+=(0); done
}

# Direct setter (no function call inside inner loop to avoid cmd-sub).
press() {
    local pr=$1 pc=$2 i j idx cur
    typeset -a positions
    positions=( "$pr $pc" )
    (( pr > 1 )) && positions+=( "$((pr-1)) $pc" )
    (( pr < SIZE )) && positions+=( "$((pr+1)) $pc" )
    (( pc > 1 )) && positions+=( "$pr $((pc-1))" )
    (( pc < SIZE )) && positions+=( "$pr $((pc+1))" )
    local p
    for p in "${positions[@]}"; do
        set -- ${=p}
        idx=$(( ($1 - 1) * SIZE + $2 ))
        cur=${BOARD[idx]}
        BOARD[idx]=$(( 1 - cur ))
    done
}

print_board() {
    local r c idx v line
    for ((r=1; r<=SIZE; r++)); do
        line=""
        for ((c=1; c<=SIZE; c++)); do
            idx=$(( (r-1) * SIZE + c ))
            v=${BOARD[idx]}
            if (( v )); then line+="● "; else line+="○ "; fi
        done
        echo "$line"
    done
}

count_on() {
    local n=0 v
    for v in "${BOARD[@]}"; do (( n += v )); done
    echo $n
}

echo "── initial (all off) ──"
init_board
print_board

echo
echo "── press (3,3) ──"
press 3 3
print_board
echo "lights on: $(count_on)"

echo
echo "── press (1,1) ──"
press 1 1
print_board

echo
echo "── press (5,5) — corner ──"
press 5 5
print_board

echo
echo "── press (1,1) again (toggles back) ──"
press 1 1
print_board

echo
echo "── manually set + solve ──"
init_board
BOARD[$(( (3-1) * SIZE + 3 ))]=1
BOARD[$(( (2-1) * SIZE + 3 ))]=1
BOARD[$(( (4-1) * SIZE + 3 ))]=1
BOARD[$(( (3-1) * SIZE + 2 ))]=1
BOARD[$(( (3-1) * SIZE + 4 ))]=1
echo "starting position:"
print_board
echo "press (3,3) to solve:"
press 3 3
print_board
echo "lights remaining: $(count_on)"

echo
echo "── random play (5 random presses, seeded) ──"
RANDOM=42
init_board
for i in 1 2 3 4 5; do
    r=$(( RANDOM % SIZE + 1 ))
    c=$(( RANDOM % SIZE + 1 ))
    echo "press ($r,$c):"
    press $r $c
    print_board
    echo "lights: $(count_on)"
    echo
done

# === ztest assertions ===
init_board
zassert_eq "$(count_on)" "0" "init: all off"
zassert_eq "${#BOARD[@]}" "25" "5x5 board has 25 cells"
press 3 3
zassert_eq "$(count_on)" "5" "press(3,3) lights up self+4 neighbours"
press 3 3
zassert_eq "$(count_on)" "0" "press(3,3) twice cancels"
init_board
press 1 1
zassert_eq "$(count_on)" "3" "press(1,1) corner: self+2 neighbours"
init_board
press 5 5
zassert_eq "$(count_on)" "3" "press(5,5) corner: self+2 neighbours"
init_board
BOARD[$(( (3-1) * SIZE + 3 ))]=1
BOARD[$(( (2-1) * SIZE + 3 ))]=1
BOARD[$(( (4-1) * SIZE + 3 ))]=1
BOARD[$(( (3-1) * SIZE + 2 ))]=1
BOARD[$(( (3-1) * SIZE + 4 ))]=1
zassert_eq "$(count_on)" "5" "plus pattern lights up"
press 3 3
zassert_eq "$(count_on)" "0" "press(3,3) clears plus pattern"
ztest_run
