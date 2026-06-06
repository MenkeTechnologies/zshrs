#!/usr/bin/env zshrs
# Nim — XOR theorem, winning strategy, scripted plays.

# Nim sum = XOR of pile sizes.
nim_sum() {
    local x=0 p
    for p in "$@"; do
        x=$(( x ^ p ))
    done
    echo $x
}

# Is current position a loss for the player to move?
# (Position is a P-position iff nim sum = 0.)
is_p_position() {
    local sum=$(nim_sum "$@")
    (( sum == 0 ))
}

# Compute optimal move: pick pile and new size.
# Returns "pile_idx new_size".
optimal_move() {
    local -a piles
    piles=("$@")
    local n=${#piles}
    local s=$(nim_sum "$@")
    if (( s == 0 )); then
        # Losing position — take 1 from largest non-empty.
        local i max=0 max_idx=0
        for ((i=1; i<=n; i++)); do
            if (( piles[i] > max )); then
                max=${piles[i]}
                max_idx=$i
            fi
        done
        if (( max > 0 )); then
            echo "$max_idx $((max - 1))"
        else
            echo ""
        fi
        return
    fi
    # Winning: find pile p such that p XOR s < p (then take p - (p XOR s)).
    local i
    for ((i=1; i<=n; i++)); do
        local p=${piles[i]}
        local new=$(( p ^ s ))
        if (( new < p )); then
            echo "$i $new"
            return
        fi
    done
}

# Random move: take 1..pile_size from a random non-empty pile.
random_move() {
    local -a piles
    piles=("$@")
    local n=${#piles}
    local -a non_empty
    non_empty=()
    local i
    for ((i=1; i<=n; i++)); do
        if (( piles[i] > 0 )); then non_empty+=($i); fi
    done
    if (( ${#non_empty} == 0 )); then echo ""; return; fi
    local idx=${non_empty[$(( RANDOM % ${#non_empty} + 1 ))]}
    local take=$(( RANDOM % piles[idx] + 1 ))
    local new=$(( piles[idx] - take ))
    echo "$idx $new"
}

print_piles() {
    local -a piles
    piles=("$@")
    local i
    for ((i=1; i<=${#piles}; i++)); do
        printf "  pile %d: " $i
        local j p=${piles[i]}
        for ((j=0; j<p; j++)); do printf "●"; done
        if (( p == 0 )); then printf "(empty)"; fi
        echo
    done
    echo "  nim sum: $(nim_sum "${piles[@]}")"
}

is_game_over() {
    local p
    for p in "$@"; do
        (( p > 0 )) && return 1
    done
    return 0
}

play_game() {
    local -a piles
    piles=("$@")
    local turn=$1
    shift
    piles=("$@")
    echo "── start ──"
    print_piles "${piles[@]}"
    echo "  player to move: $turn"

    local rounds=0
    while ! is_game_over "${piles[@]}"; do
        (( rounds++ ))
        echo
        echo "── round $rounds: $turn ──"
        local move
        if [[ $turn == OPT ]]; then
            move=$(optimal_move "${piles[@]}")
        else
            move=$(random_move "${piles[@]}")
        fi
        if [[ -z $move ]]; then break; fi
        local idx=${move%% *}
        local new=${move##* }
        local taken=$(( piles[idx] - new ))
        echo "  $turn takes $taken from pile $idx (now $new)"
        piles[idx]=$new
        print_piles "${piles[@]}"
        if is_game_over "${piles[@]}"; then break; fi
        if [[ $turn == OPT ]]; then turn=RAND; else turn=OPT; fi
    done

    # In normal play, last to move wins.
    if [[ $turn == OPT ]]; then
        echo
        echo "  ✓ OPT (optimal) takes last → WINS"
    else
        echo
        echo "  ✗ RAND took last → wins"
    fi
}

echo "── Nim theorem (P-positions) ──"
positions=(
    "3 4 5"   # sum = 2 (N-position, winnable)
    "1 2 3"   # sum = 0 (P-position, losing)
    "5 5"     # sum = 0
    "1 1 1 1" # sum = 0
    "7 7 7"   # sum = 7
    "1 4 5"   # sum = 0
)
for p in "${positions[@]}"; do
    s=$(nim_sum ${=p})
    if (( s == 0 )); then
        echo "  ($p): nim sum = 0 → P-position (current player LOSES with optimal play)"
    else
        echo "  ($p): nim sum = $s → N-position (current player WINS)"
    fi
done

echo
echo "── optimal moves ──"
for p in "${positions[@]}"; do
    move=$(optimal_move ${=p})
    if [[ -z $move ]]; then
        echo "  ($p): game already over"
    else
        idx=${move%% *}
        new=${move##* }
        piles_arr=(${=p})
        taken=$(( piles_arr[idx] - new ))
        echo "  ($p): take $taken from pile $idx → new size $new"
    fi
done

echo
echo "── full game: OPT vs RAND ──"
RANDOM=42
play_game OPT 3 4 5

echo
echo
echo "── 10 games: OPT vs RAND from (3,5,7) ──"
RANDOM=42
opt_wins=0
rand_wins=0
for g in {1..10}; do
    local -a piles
    piles=(3 5 7)
    local turn=OPT
    while ! is_game_over "${piles[@]}"; do
        local move
        if [[ $turn == OPT ]]; then
            move=$(optimal_move "${piles[@]}")
        else
            move=$(random_move "${piles[@]}")
        fi
        [[ -z $move ]] && break
        idx=${move%% *}
        new=${move##* }
        piles[idx]=$new
        if [[ $turn == OPT ]]; then turn=RAND; else turn=OPT; fi
    done
    # Whoever moved LAST is the one who would move "next" in turn variable.
    if [[ $turn == RAND ]]; then
        (( opt_wins++ ))
    else
        (( rand_wins++ ))
    fi
done
echo "  OPT wins: $opt_wins / 10 (should be 10 from N-position)"
echo "  RAND wins: $rand_wins / 10"

echo
echo "── proof: OPT always wins from N-position ──"
echo "  Nim sum (3,5,7) = $(nim_sum 3 5 7)"
echo "  Strategy: keep nim sum = 0 after every OPT move"
echo "  → RAND can never restore sum to 0"
echo "  → OPT always responds, takes last pile"

# === ztest assertions ===
zassert_eq "$(nim_sum 3 4 5)" 2 "nim sum 3,4,5"
zassert_eq "$(nim_sum 1 2 3)" 0 "nim sum 1,2,3 = P-pos"
zassert_eq "$(nim_sum 5 5)"   0 "twin piles"
zassert_eq "$(nim_sum 1 4 5)" 0 "1,4,5 = P-pos"
zassert_eq "$(nim_sum 3 5 7)" 1 "3,5,7 N-pos"
if is_p_position 1 2 3; then zassert_ok 1 "1,2,3 is P"
else zassert_ok 0 "1,2,3 should be P"; fi
if is_p_position 3 4 5; then zassert_ok 0 "3,4,5 should not be P"
else zassert_ok 1 "3,4,5 is N"; fi
if is_game_over 0 0 0; then zassert_ok 1 "all zero = over"
else zassert_ok 0 "all zero should be over"; fi
zassert_eq "$opt_wins" 10 "OPT wins all 10"
ztest_run
