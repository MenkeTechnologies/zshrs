#!/usr/bin/env zshrs
# Blackjack — scripted dealer + player decisions.

# Card value (Ace=1 or 11 handled in hand_value).
card_value() {
    local c=$1
    case $c in
        A) echo 1 ;;     # handled specially
        K|Q|J) echo 10 ;;
        *) echo $c ;;
    esac
}

# Compute best hand value w/ Ace soft/hard logic.
hand_value() {
    local -a hand
    hand=("$@")
    local total=0 aces=0 c v
    for c in "${hand[@]}"; do
        v=$(card_value $c)
        (( total += v ))
        [[ $c == A ]] && (( aces++ ))
    done
    # Upgrade aces 1→11 if still under 21.
    while (( aces > 0 && total + 10 <= 21 )); do
        (( total += 10 ))
        (( aces-- ))
    done
    echo $total
}

# Build standard deck.
make_deck() {
    local -a deck
    local rank suit
    local ranks=(A 2 3 4 5 6 7 8 9 10 J Q K)
    local suits=(♠ ♥ ♦ ♣)
    for suit in "${suits[@]}"; do
        for rank in "${ranks[@]}"; do
            deck+=("${rank}${suit}")
        done
    done
    echo "${deck[@]}"
}

# Fisher-Yates deterministic with seed.
shuffle() {
    local -a arr
    arr=("$@")
    local n=${#arr} i j tmp
    for ((i=n; i>1; i--)); do
        j=$(( RANDOM % i + 1 ))
        tmp=${arr[i]}
        arr[i]=${arr[j]}
        arr[j]=$tmp
    done
    echo "${arr[@]}"
}

# Extract rank from card (strip suit).
rank_of() {
    local c=$1
    if [[ $c == 10* ]]; then echo "10"
    else echo "${c[1]}"
    fi
}

play_round() {
    local round=$1
    shift
    local -a deck
    deck=("$@")
    local idx=1
    local -a phand dhand
    # Deal alternating.
    phand+=("${deck[idx]}"); ((idx++))
    dhand+=("${deck[idx]}"); ((idx++))
    phand+=("${deck[idx]}"); ((idx++))
    dhand+=("${deck[idx]}"); ((idx++))
    echo "── round $round ──"

    # Player ranks for hand_value
    local -a p_ranks d_ranks
    p_ranks=()
    d_ranks=()
    for c in "${phand[@]}"; do p_ranks+=( $(rank_of $c) ); done
    for c in "${dhand[@]}"; do d_ranks+=( $(rank_of $c) ); done

    echo "  player: ${phand[*]} = $(hand_value $p_ranks)"
    echo "  dealer: ${dhand[1]} ?"

    # Player strategy: hit while < 17.
    while (( $(hand_value $p_ranks) < 17 )); do
        phand+=("${deck[idx]}")
        p_ranks+=( $(rank_of "${deck[idx]}") )
        (( idx++ ))
        v=$(hand_value $p_ranks)
        echo "  player hits: ${phand[-1]} → ${phand[*]} = $v"
        if (( v > 21 )); then
            echo "  ✗ player BUSTS"
            return 1
        fi
    done
    local pv=$(hand_value $p_ranks)
    echo "  player stands at $pv"

    # Dealer reveals & hits to 17.
    echo "  dealer reveals: ${dhand[*]} = $(hand_value $d_ranks)"
    while (( $(hand_value $d_ranks) < 17 )); do
        dhand+=("${deck[idx]}")
        d_ranks+=( $(rank_of "${deck[idx]}") )
        (( idx++ ))
        v=$(hand_value $d_ranks)
        echo "  dealer hits: ${dhand[-1]} → ${dhand[*]} = $v"
        if (( v > 21 )); then
            echo "  ✓ dealer BUSTS — player wins!"
            return 0
        fi
    done
    local dv=$(hand_value $d_ranks)
    echo "  dealer stands at $dv"

    # Compare.
    if (( pv > dv )); then
        echo "  ✓ player wins ($pv vs $dv)"
        return 0
    elif (( pv < dv )); then
        echo "  ✗ dealer wins ($dv vs $pv)"
        return 1
    else
        echo "  = push ($pv each)"
        return 2
    fi
}

echo "=== Blackjack simulator (3 rounds, seeded) ==="
RANDOM=42
deck_str=$(make_deck)
deck=( ${=deck_str} )
echo "deck size: ${#deck}"

wins=0
losses=0
pushes=0
for round in 1 2 3; do
    shuf_str=$(shuffle "${deck[@]}")
    shuf=( ${=shuf_str} )
    play_round $round "${shuf[@]}"
    case $? in
        0) (( wins++ )) ;;
        1) (( losses++ )) ;;
        2) (( pushes++ )) ;;
    esac
    echo
done

echo "── totals ──"
echo "  wins:   $wins"
echo "  losses: $losses"
echo "  pushes: $pushes"

echo
echo "── hand-value edge cases ──"
echo "  A,K     = $(hand_value A K)  (blackjack-ish, 21 if 1st card)"
echo "  A,A,9   = $(hand_value A A 9)"
echo "  A,5,Q   = $(hand_value A 5 10)"
echo "  K,Q,J   = $(hand_value K Q J)"
echo "  A,A,A,A = $(hand_value A A A A)"

# === ztest assertions ===
# Hand-value edge cases (deterministic regardless of RANDOM seed).
zassert_eq "$(hand_value A K)"       21  "A+K = 21 (Ace soft)"
zassert_eq "$(hand_value A A 9)"     21  "A+A+9 = 21 (one ace soft)"
zassert_eq "$(hand_value A 5 10)"    16  "A+5+10 = 16 (ace forced hard)"
zassert_eq "$(hand_value K Q J)"     30  "K+Q+J = 30 (no aces)"
zassert_eq "$(hand_value A A A A)"   14  "four aces = 11+1+1+1"
zassert_eq "$(hand_value 2 3 4)"     9   "low cards sum"
zassert_eq "$(hand_value A 9)"       20  "A+9 = 20"
# card_value spot checks
zassert_eq "$(card_value A)"   1  "Ace base value 1"
zassert_eq "$(card_value K)"   10 "King = 10"
zassert_eq "$(card_value 7)"   7  "numeric card"
# Deck has 52 cards
zassert_eq "${#deck}"  52  "standard deck = 52 cards"
# rank_of
zassert_eq "$(rank_of A♠)"  "A"  "rank_of strips suit (A)"
zassert_eq "$(rank_of 10♥)" "10" "rank_of preserves 10"
ztest_run
