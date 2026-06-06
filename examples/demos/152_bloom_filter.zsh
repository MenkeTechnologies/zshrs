#!/usr/bin/env zshrs
# Toy Bloom filter — uses simple hash on strings.

typeset -A BLOOM_BITS
typeset -i BLOOM_SIZE=256

# Simple deterministic hash on a string (sum of byte values * prime).
str_hash() {
    local s=$1 i ch sum=0
    for ((i = 1; i <= ${#s}; i++)); do
        ch=${s[i]}
        local code=$(printf "%d" "'$ch")
        sum=$(( (sum * 31 + code) & 0xFFFFFFFF ))
    done
    echo $sum
}

bloom_add() {
    local item=$1
    local h1=$(str_hash "$item")
    local h2=$(str_hash "salt$item")
    local h3=$(str_hash "${item}pepper")
    BLOOM_BITS[$(( h1 % BLOOM_SIZE ))]=1
    BLOOM_BITS[$(( h2 % BLOOM_SIZE ))]=1
    BLOOM_BITS[$(( h3 % BLOOM_SIZE ))]=1
}

bloom_check() {
    local item=$1
    local h1=$(str_hash "$item")
    local h2=$(str_hash "salt$item")
    local h3=$(str_hash "${item}pepper")
    if [[ -z ${BLOOM_BITS[$(( h1 % BLOOM_SIZE ))]+x} ]]; then echo "no"; return; fi
    if [[ -z ${BLOOM_BITS[$(( h2 % BLOOM_SIZE ))]+x} ]]; then echo "no"; return; fi
    if [[ -z ${BLOOM_BITS[$(( h3 % BLOOM_SIZE ))]+x} ]]; then echo "no"; return; fi
    echo "maybe"
}

echo "── add ──"
for w in apple banana cherry date elderberry fig grape; do
    bloom_add "$w"
done
echo "bits set: ${#BLOOM_BITS[@]} of $BLOOM_SIZE"

echo "── check inserted ──"
for w in apple cherry grape; do
    echo "  $w: $(bloom_check "$w")"
done

echo "── check non-inserted ──"
for w in xyz watermelon hello world; do
    echo "  $w: $(bloom_check "$w")"
done

echo "── false-positive rate test ──"
total=0; positives=0
for w in zzz aaa bbb ccc ddd eee fff hhh iii jjj kkk lll mmm; do
    (( total++ ))
    if [[ $(bloom_check "$w") == maybe ]]; then
        (( positives++ ))
    fi
done
echo "tested $total uninserted; $positives false positives"

# === ztest assertions ===
zassert_eq "$(bloom_check apple)"      "maybe" "inserted apple -> maybe"
zassert_eq "$(bloom_check cherry)"     "maybe" "inserted cherry -> maybe"
zassert_eq "$(bloom_check grape)"      "maybe" "inserted grape -> maybe"
zassert_eq "$(bloom_check watermelon)" "no"    "uninserted watermelon -> no"
zassert_eq "$(bloom_check xyz)"        "no"    "uninserted xyz -> no"
zassert_eq "$total"     "13"  "total uninserted = 13"
zassert_eq "$positives" "0"   "0 false positives for this corpus"
# Add idempotency: re-add doesn't change bit count or check result.
bits_before=${#BLOOM_BITS[@]}
bloom_add apple
zassert_eq "${#BLOOM_BITS[@]}" "$bits_before" "re-adding apple keeps bit count"
zassert_eq "$(bloom_check apple)" "maybe" "apple still maybe"
# str_hash determinism
h1=$(str_hash "foo")
h2=$(str_hash "foo")
zassert_eq "$h1" "$h2" "str_hash deterministic"
ztest_run
