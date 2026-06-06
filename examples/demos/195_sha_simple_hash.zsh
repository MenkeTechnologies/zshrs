#!/usr/bin/env zshrs
# Toy hash functions in pure zsh — not cryptographically secure.

# Polynomial rolling hash (Rabin-Karp style).
poly_hash() {
    local s=$1
    local hash=0 i ch code
    local prime=257
    local mod=$(( 1 << 30 ))
    for ((i=1; i<=${#s}; i++)); do
        ch=${s[i]}
        code=$(printf "%d" "'$ch")
        hash=$(( (hash * prime + code) % mod ))
    done
    printf "%08x\n" $hash
}

# DJB2 hash.
djb2_hash() {
    local s=$1
    local hash=5381 i ch code
    for ((i=1; i<=${#s}; i++)); do
        ch=${s[i]}
        code=$(printf "%d" "'$ch")
        hash=$(( ((hash << 5) + hash + code) & 0xFFFFFFFF ))
    done
    printf "%08x\n" $hash
}

# FNV-1a 32-bit.
fnv1a() {
    local s=$1
    local hash=2166136261 i ch code
    local prime=16777619
    local mod=$(( 1 << 32 ))
    for ((i=1; i<=${#s}; i++)); do
        ch=${s[i]}
        code=$(printf "%d" "'$ch")
        hash=$(( (hash ^ code) * prime % mod ))
    done
    printf "%08x\n" $hash
}

# Adler-32 (used in zlib).
adler32() {
    local s=$1
    local a=1 b=0 i ch code
    local mod=65521
    for ((i=1; i<=${#s}; i++)); do
        ch=${s[i]}
        code=$(printf "%d" "'$ch")
        a=$(( (a + code) % mod ))
        b=$(( (b + a) % mod ))
    done
    printf "%08x\n" $(( (b << 16) | a ))
}

echo "── poly hash ──"
for s in "" "a" "abc" "hello" "hello world" "The quick brown fox"; do
    printf "  '%s' → %s\n" "$s" "$(poly_hash "$s")"
done

echo "── djb2 ──"
for s in "" "a" "abc" "hello" "hello world"; do
    printf "  '%s' → %s\n" "$s" "$(djb2_hash "$s")"
done

echo "── fnv1a ──"
for s in "" "a" "abc" "hello" "hello world"; do
    printf "  '%s' → %s\n" "$s" "$(fnv1a "$s")"
done

echo "── adler-32 ──"
for s in "" "a" "abc" "hello" "Wikipedia"; do
    printf "  '%s' → %s\n" "$s" "$(adler32 "$s")"
done

echo "── consistency check (same input → same hash) ──"
input="determinism check"
h1=$(poly_hash "$input")
h2=$(poly_hash "$input")
[[ "$h1" == "$h2" ]] && echo "  poly: stable ($h1)"
h1=$(djb2_hash "$input")
h2=$(djb2_hash "$input")
[[ "$h1" == "$h2" ]] && echo "  djb2: stable ($h1)"

echo "── collision-resistance smoke test ──"
typeset -A hash_set
for s in apple banana cherry date elderberry fig grape honeydew kiwi lemon; do
    h=$(djb2_hash "$s")
    hash_set[$h]=$s
done
echo "  hashed: 10 unique inputs"
echo "  unique hashes: ${#hash_set[@]}"

# === ztest assertions ===
# Known hash values (computed by the implementation, asserted here as
# regression pins — change them only if the algorithm itself changes).
zassert_eq "$(poly_hash '')"     "00000000" "poly empty"
zassert_eq "$(poly_hash 'a')"    "00000061" "poly 'a'"
zassert_eq "$(poly_hash 'abc')"  "00622526" "poly 'abc'"
zassert_eq "$(djb2_hash '')"     "00001505" "djb2 empty (5381)"
zassert_eq "$(djb2_hash 'abc')"  "0b885c8b" "djb2 'abc'"
zassert_eq "$(fnv1a '')"         "811c9dc5" "fnv1a empty (offset basis)"
zassert_eq "$(adler32 '')"       "00000001" "adler32 empty"
zassert_eq "$(adler32 'Wikipedia')" "11e60398" "adler32 Wikipedia"
# Determinism: same input → same hash
zassert_eq "$(djb2_hash 'check')" "$(djb2_hash 'check')" "djb2 deterministic"
zassert_eq "${#hash_set[@]}" 10 "10 unique djb2 hashes for 10 inputs"
ztest_run
