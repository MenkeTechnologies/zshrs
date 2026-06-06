#!/usr/bin/env zshrs
# ASCII histogram — bar chart for a frequency map.

histogram() {
    local -a labels=()
    local -a counts=()
    while (( $# > 0 )); do
        labels+=("$1")
        counts+=("$2")
        shift 2
    done
    local max=0 v
    for v in "${counts[@]}"; do
        (( v > max )) && max=$v
    done
    local i j n
    for ((i = 1; i <= ${#labels[@]}; i++)); do
        n=${counts[i]}
        printf "%-10s " "${labels[i]}"
        for ((j = 0; j < n; j++)); do printf "#"; done
        printf " (%d)\n" $n
    done
}

echo "── distribution ──"
histogram \
    alpha 3 \
    beta 7 \
    gamma 12 \
    delta 5 \
    epsilon 9 \
    zeta 2 \
    eta 11

echo "── frequency from text ──"
text="the quick brown fox jumps over the lazy dog the fox"
typeset -A freq
for w in $=text; do
    (( freq[$w]++ ))
done
# Convert assoc → flat key/val pairs for histogram().
args=()
for k v in "${(@kv)freq}"; do
    args+=("$k" "$v")
done
histogram "${args[@]}" | sort

# === ztest assertions ===
zassert_eq "${freq[the]}"    3 "'the' appears 3 times"
zassert_eq "${freq[fox]}"    2 "'fox' appears 2 times"
zassert_eq "${freq[quick]}"  1 "'quick' appears once"
zassert_eq "${freq[dog]}"    1 "'dog' appears once"
zassert_eq "${#freq[@]}"     8 "8 unique words in text"
# Render a known histogram and check shape.
out=$(histogram a 3 b 5 2>&1)
zassert_contains "$out" "a          ### (3)" "histogram bar row a"
zassert_contains "$out" "b          ##### (5)" "histogram bar row b"
ztest_run
