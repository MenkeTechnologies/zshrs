#!/usr/bin/env zshrs
# Bloom filter — multi-hash bit array w/ false-positive analysis.

# Configuration: 256 bits, 3 hash functions.
BITS=256
HASHES=3
typeset -a BIT_ARRAY

bf_init() {
    BIT_ARRAY=()
    local i
    for ((i=1; i<=BITS; i++)); do BIT_ARRAY[i]=0; done
}

# Polynomial hash with seed.
hash_pos() {
    local s=$1 seed=$2
    local h=$seed i ch ord
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        ord=$(( #ch ))
        h=$(( (h * 31 + ord) % BITS ))
    done
    echo $(( h + 1 ))
}

bf_add() {
    local s=$1 seed i p
    for ((seed=1; seed<=HASHES; seed++)); do
        p=$(hash_pos "$s" $seed)
        BIT_ARRAY[p]=1
    done
}

bf_contains() {
    local s=$1 seed p
    for ((seed=1; seed<=HASHES; seed++)); do
        p=$(hash_pos "$s" $seed)
        (( BIT_ARRAY[p] == 0 )) && return 1
    done
    return 0
}

bf_count_set() {
    local n=0 i
    for ((i=1; i<=BITS; i++)); do
        (( n += BIT_ARRAY[i] ))
    done
    echo $n
}

echo "── add 20 known words ──"
bf_init
words=(
    apple banana cherry date elderberry fig grape honeydew
    kiwi lemon mango nectarine orange papaya quince raspberry
    strawberry tangerine ugli vanilla
)
for w in "${words[@]}"; do
    bf_add "$w"
done
echo "  added: ${#words} words"
echo "  bits set: $(bf_count_set) / $BITS"
echo "  fill rate: $(( $(bf_count_set) * 100 / BITS ))%"

echo
echo "── true positive check (all added words) ──"
fp=0
for w in "${words[@]}"; do
    if ! bf_contains "$w"; then
        echo "  ✗ false negative: $w"
        (( fp++ ))
    fi
done
if (( fp == 0 )); then
    echo "  ✓ all ${#words} added words detected (no false negatives — guaranteed)"
fi

echo
echo "── false positive check (50 not-added words) ──"
not_added=(
    avocado blueberry cantaloupe dragonfruit eggplant
    fennel guava huckleberry indianfig jackfruit
    kumquat loquat melon nut olive
    peach pomegranate plum pineapple persimmon
    rhubarb soursop tomato tamarind ugni
    voavanga wampi xigua yellowapple zucchini
    coconut durian ackee bilimbi calabash
    cudrang dingleberry feijoa gac hala
    imbe jambolan kaffirlime lulo medlar
    naranjilla otaheite pawpaw pequi quararibea
)
fpos=0
for w in "${not_added[@]}"; do
    if bf_contains "$w"; then
        (( fpos++ ))
    fi
done
echo "  false positives: $fpos / ${#not_added}"
fp_rate=$(( fpos * 1000 / ${#not_added} ))
echo "  observed FP rate: ${fp_rate}/1000 ≈ ${fp_rate}/10%"

echo
echo "── theoretical FP rate ──"
# FP rate ≈ (1 - e^(-kn/m))^k where k=hashes, n=items, m=bits.
echo "  formula: (1 - e^(-kn/m))^k"
echo "  k=$HASHES (hashes), n=${#words} (items), m=$BITS (bits)"
echo "  rough estimate: bits-set-rate=$(( $(bf_count_set) * 100 / BITS ))% so FP ≈ (set-rate)^k"
set_rate=$(bf_count_set)
sr=$(( set_rate * 100 / BITS ))
est=$(( sr * sr * sr / 10000 ))
echo "  ≈ ${sr}^3/10000 = ${est}%"

echo
echo "── union of two filters ──"
typeset -a FILTER_A FILTER_B
# Filter A: even words.
BIT_ARRAY=(); bf_init
for ((i=1; i<=${#words}; i+=2)); do
    bf_add "${words[i]}"
done
FILTER_A=("${BIT_ARRAY[@]}")
echo "  Filter A (odd-indexed words): bits set = $(bf_count_set)"

bf_init
for ((i=2; i<=${#words}; i+=2)); do
    bf_add "${words[i]}"
done
FILTER_B=("${BIT_ARRAY[@]}")
echo "  Filter B (even-indexed words): bits set = $(bf_count_set)"

# Union (bitwise OR).
typeset -a UNION
for ((i=1; i<=BITS; i++)); do
    if (( FILTER_A[i] || FILTER_B[i] )); then
        UNION[i]=1
    else
        UNION[i]=0
    fi
done
union_count=0
for ((i=1; i<=BITS; i++)); do (( union_count += UNION[i] )); done
echo "  Union: bits set = $union_count"

# Intersection (bitwise AND).
typeset -a INTER
for ((i=1; i<=BITS; i++)); do
    if (( FILTER_A[i] && FILTER_B[i] )); then
        INTER[i]=1
    else
        INTER[i]=0
    fi
done
inter_count=0
for ((i=1; i<=BITS; i++)); do (( inter_count += INTER[i] )); done
echo "  Intersection: bits set = $inter_count"

echo
echo "── space efficiency ──"
echo "  ${#words} items in ${BITS} bits = $(( BITS / 8 )) bytes total"
echo "  per item: $(( BITS / 8 / ${#words} )) bytes (vs ~10+ for hash set)"

# === ztest assertions ===
bf_init
zassert_eq "$(bf_count_set)" "0"           "init: zero bits set"
for w in apple banana cherry; do bf_add "$w"; done
zassert_gt "$(bf_count_set)" "0"           "after adds: bits set"
bf_contains apple && r=1 || r=0
zassert_eq "$r" "1"                        "no false negative for apple"
bf_contains banana && r=1 || r=0
zassert_eq "$r" "1"                        "no false negative for banana"
bf_contains cherry && r=1 || r=0
zassert_eq "$r" "1"                        "no false negative for cherry"
# After 20 adds the bits-set count should match observed value
bf_init
for w in apple banana cherry date elderberry fig grape honeydew kiwi lemon mango nectarine orange papaya quince raspberry strawberry tangerine ugli vanilla; do
    bf_add "$w"
done
n=$(bf_count_set)
zassert_le "$n" "60"                       "20 items × 3 hashes set ≤ 60 bits"
zassert_ge "$n" "30"                       "≥ 30 bits set"
ztest_run
