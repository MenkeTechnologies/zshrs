#!/usr/bin/env zshrs
# Deck of cards — generation, shuffle, deal.

ranks=(2 3 4 5 6 7 8 9 T J Q K A)
suits=(♠ ♥ ♦ ♣)

build_deck() {
    deck=()
    for r in $ranks; do
        for s in $suits; do
            deck+=("${r}${s}")
        done
    done
}

shuffle_deck() {
    # Fisher-Yates.
    local n=${#deck[@]} i j tmp
    for ((i=n; i>1; i--)); do
        j=$(( RANDOM % i + 1 ))
        tmp=${deck[i]}
        deck[i]=${deck[j]}
        deck[j]=$tmp
    done
}

deal_hand() {
    local n=$1
    local hand=("${deck[@]:0:$n}")
    deck=("${deck[@]:$n}")
    echo "${hand[@]}"
}

echo "── new deck ──"
build_deck
echo "size: ${#deck[@]}"
echo "first 5: ${deck[@]:0:5}"
echo "last 5: ${deck[@]:(-5)}"

echo
echo "── shuffle deck (seed=42) ──"
RANDOM=42
build_deck
shuffle_deck
echo "first 5 after shuffle: ${deck[@]:0:5}"

echo
echo "── deal 4 hands of 5 ──"
build_deck
RANDOM=100
shuffle_deck
for i in 1 2 3 4; do
    echo "  player $i: $(deal_hand 5)"
done
echo "  deck remaining: ${#deck[@]}"

echo
echo "── poker hand classification (simplified) ──"
classify_poker() {
    local -a hand=("$@")
    local -A rank_count
    for c in "${hand[@]}"; do
        local r=${c[1]}
        (( rank_count[$r]++ ))
    done
    local pairs=0 trips=0 quads=0
    for k v in "${(@kv)rank_count}"; do
        if (( v == 2 )); then (( pairs++ )); fi
        if (( v == 3 )); then (( trips++ )); fi
        if (( v == 4 )); then (( quads++ )); fi
    done
    if (( quads )); then echo "four of a kind"
    elif (( trips && pairs )); then echo "full house"
    elif (( trips )); then echo "three of a kind"
    elif (( pairs == 2 )); then echo "two pair"
    elif (( pairs == 1 )); then echo "one pair"
    else echo "high card"
    fi
}

echo "AA KK Q:    $(classify_poker A♠ A♥ K♠ K♥ Q♦)"
echo "AAA 22:     $(classify_poker A♠ A♥ A♦ 2♠ 2♥)"
echo "AAAA K:     $(classify_poker A♠ A♥ A♦ A♣ K♠)"
echo "AKQJT:      $(classify_poker A♠ K♥ Q♦ J♣ T♠)"

echo
echo "── card iteration: rank histogram ──"
RANDOM=7
build_deck
shuffle_deck
deal_hand 26 > /dev/null  # discard half
typeset -A rh
for c in "${deck[@]}"; do
    (( rh[${c[1]}]++ ))
done
for r in $ranks; do
    printf "  %s: %d\n" $r ${rh[$r]:-0}
done

# === ztest assertions ===
build_deck
zassert_eq "${#deck[@]}" 52   "new deck has 52 cards"
zassert_eq "${deck[1]}"  "2♠"  "first card is 2♠"
zassert_eq "${deck[52]}" "A♣"  "last card is A♣"
zassert_eq "$(classify_poker A♠ A♥ K♠ K♥ Q♦)" "two pair"        "two pair"
zassert_eq "$(classify_poker A♠ A♥ A♦ 2♠ 2♥)" "full house"      "full house"
zassert_eq "$(classify_poker A♠ A♥ A♦ A♣ K♠)" "four of a kind"  "four of a kind"
zassert_eq "$(classify_poker A♠ K♥ Q♦ J♣ T♠)" "high card"       "high card"
zassert_eq "$(classify_poker A♠ A♥ K♠ Q♥ J♦)" "one pair"        "one pair"
zassert_eq "$(classify_poker A♠ A♥ A♦ K♠ Q♥)" "three of a kind" "three of a kind"
ztest_run
