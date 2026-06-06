#!/usr/bin/env zshrs
# Memory-match (concentration) — 4×4 grid, scripted plays.

SIZE=4
typeset -a board revealed

# Fixed seed deck (8 pairs).
init_board() {
    local symbols=(A A B B C C D D E E F F G G H H)
    # Deterministic "shuffle": index-based permutation.
    local order=(2 9 5 14 1 10 7 13 4 11 8 16 3 12 6 15)
    board=()
    local i
    for ((i=1; i<=16; i++)); do
        board[i]=${symbols[ order[i] ]}
    done
    revealed=()
    for ((i=1; i<=16; i++)); do revealed[i]=0; done
}

show_board() {
    local r c idx
    for ((r=0; r<SIZE; r++)); do
        for ((c=0; c<SIZE; c++)); do
            idx=$((r * SIZE + c + 1))
            if (( revealed[idx] )); then
                printf " %s " "${board[idx]}"
            else
                printf "[%2d]" $idx
            fi
        done
        echo
    done
}

flip() {
    local a=$1 b=$2
    local va=${board[a]} vb=${board[b]}
    revealed[a]=1
    revealed[b]=1
    printf "  flip (%d,%d): %s %s   " $a $b $va $vb
    if [[ $va == $vb ]]; then
        echo "MATCH ✓"
        return 0
    else
        echo "no match"
        revealed[a]=0
        revealed[b]=0
        return 1
    fi
}

count_revealed() {
    local n=0 i
    for ((i=1; i<=16; i++)); do
        (( revealed[i] )) && (( n++ ))
    done
    echo $n
}

init_board

echo "── initial board (hidden) ──"
show_board

echo
echo "── reveal answer key (for demo) ──"
local i
for ((i=1; i<=16; i++)); do revealed[i]=1; done
show_board
for ((i=1; i<=16; i++)); do revealed[i]=0; done

# Play: precomputed pairs from the deck above.
# Deck positions: A@5,A@1  B@1->4,7... use known pairs.
# Just walk pairs that match.
echo
echo "── play known matches ──"
# Find pairs by scanning.
typeset -A first_seen
declare -a pairs
for ((i=1; i<=16; i++)); do
    local s=${board[i]}
    if (( ${+first_seen[$s]} )); then
        pairs+=( "${first_seen[$s]} $i" )
    else
        first_seen[$s]=$i
    fi
done

attempts=0
matches=0
for p in "${pairs[@]}"; do
    set -- $=p
    if flip $1 $2; then (( matches++ )); fi
    (( attempts++ ))
done

echo
echo "── final state ──"
show_board
echo
echo "  attempts: $attempts"
echo "  matches:  $matches"
echo "  revealed: $(count_revealed)/16"

echo
echo "── play with misses (alternating wrong) ──"
init_board
attempts=0
matches=0
# Try (1,2), (3,4), (5,6) — likely misses unless symbols coincide.
for pair in "1 2" "3 4" "5 6" "7 8" "9 10"; do
    set -- $=pair
    if flip $1 $2; then (( matches++ )); fi
    (( attempts++ ))
done
echo "  matches: $matches/$attempts"

# === ztest assertions ===
init_board
zassert_eq "${#board[@]}" "16"     "16 cards"
zassert_eq "${#revealed[@]}" "16"  "16 revealed flags"
zassert_eq "$(count_revealed)" "0" "all hidden initially"
# Index the deck against the known fixed permutation and pick a real pair.
# symbols=(A A B B C C D D E E F F G G H H), order=(2 9 5 14 1 10 7 13 4 11 8 16 3 12 6 15)
# board[1]=symbols[2]=A, board[5]=symbols[1]=A → flip 1,5 must match.
if flip 1 5; then zassert_ok 1 "board[1] == board[5] (both A)"; else zassert_ok 0 "board[1] == board[5]"; fi
# After matching, two cells revealed
zassert_eq "$(count_revealed)" "2" "match reveals 2 cells"
# Non-match should NOT leave cells revealed.
init_board
if flip 1 2; then zassert_ok 0 "1,2 not a match"; else zassert_ok 1 "1,2 not a match"; fi
zassert_eq "$(count_revealed)" "0" "miss hides both again"
# Sanity check deck distribution: each of A..H appears twice.
total_A=0
for i in {1..16}; do [[ ${board[i]} == A ]] && (( total_A++ )); done
zassert_eq "$total_A" "2" "A appears exactly twice"
total_H=0
for i in {1..16}; do [[ ${board[i]} == H ]] && (( total_H++ )); done
zassert_eq "$total_H" "2" "H appears exactly twice"
ztest_run
