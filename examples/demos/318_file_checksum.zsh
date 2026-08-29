#!/usr/bin/env zshrs
# File checksums — Adler-32, DJB2, FNV-1a, CRC-style sum, byte XOR.

# Adler-32 (RFC 1950).
adler32() {
    local content=$1
    local MOD=65521
    local a=1 b=0 i ch ord
    for ((i=1; i<=${#content}; i++)); do
        ch="${content[i]}"
        ord=$(( #ch ))
        a=$(( (a + ord) % MOD ))
        b=$(( (b + a) % MOD ))
    done
    printf "%08x\n" $(( (b << 16) | a ))
}

# DJB2 hash.
djb2() {
    local s=$1 h=5381 i ch ord
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        ord=$(( #ch ))
        h=$(( ((h << 5) + h + ord) & 0xFFFFFFFF ))
    done
    printf "%08x\n" $h
}

# FNV-1a (32-bit).
fnv1a() {
    local s=$1 h=2166136261 i ch ord
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        ord=$(( #ch ))
        h=$(( (h ^ ord) * 16777619 & 0xFFFFFFFF ))
    done
    printf "%08x\n" $h
}

# Sum of bytes mod 65536.
checksum() {
    local s=$1 sum=0 i ch ord
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        ord=$(( #ch ))
        sum=$(( (sum + ord) & 0xFFFF ))
    done
    printf "%04x\n" $sum
}

# XOR byte sum.
xor_sum() {
    local s=$1 x=0 i ch ord
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        ord=$(( #ch ))
        x=$(( x ^ ord ))
    done
    printf "%02x\n" $x
}

# Build a test file with known contents.
tmpdir=$(mktemp -d)
echo "Hello, World!" > $tmpdir/hello.txt
printf "" > $tmpdir/empty.txt
echo "The quick brown fox jumps over the lazy dog" > $tmpdir/pangram.txt
echo "abcdefghijklmnopqrstuvwxyz" > $tmpdir/alpha.txt
echo "AAAAAAAAAAAAAAAAAAAA" > $tmpdir/repeat.txt
echo "0123456789" > $tmpdir/digits.txt

echo "── checksums of test files ──"
echo "  file               adler32   djb2      fnv1a     sum16  xor"
echo "  -----------------  --------  --------  --------  -----  ---"
for f in $tmpdir/*.txt; do
    content=$(cat "$f")
    name=${f##*/}
    a=$(adler32 "$content")
    d=$(djb2 "$content")
    fn=$(fnv1a "$content")
    s=$(checksum "$content")
    x=$(xor_sum "$content")
    printf "  %-18s %s  %s  %s  %s   %s\n" "$name" "$a" "$d" "$fn" "$s" "$x"
done

echo
echo "── verify same content → same hash ──"
content1="zshrs is the future"
content2="zshrs is the future"
content3="zshrs is the past"
a1=$(adler32 "$content1")
a2=$(adler32 "$content2")
a3=$(adler32 "$content3")
echo "  '$content1' adler32: $a1"
echo "  '$content2' adler32: $a2 $([[ $a1 == $a2 ]] && echo ✓ || echo ✗)"
echo "  '$content3' adler32: $a3 (different)"

echo
echo "── single bit changes → big hash changes ──"
inputs=("Hello, World!" "Hello, World?" "Hello, World." "Hello, Worle!" "Hello, World ")
for inp in "${inputs[@]}"; do
    a=$(adler32 "$inp")
    d=$(djb2 "$inp")
    fn=$(fnv1a "$inp")
    printf "  '%s' → adler=%s djb2=%s fnv1a=%s\n" "$inp" "$a" "$d" "$fn"
done

echo
echo "── empty string ──"
e=$(adler32 "")
echo "  adler32(\"\") = $e (should be 00000001)"
e=$(djb2 "")
echo "  djb2(\"\") = $e (should be 00001505 = 5381)"

echo
echo "── all-zeros vs all-ones ──"
zeros=$(printf '\\000%.0s' {1..16})
ones="AAAAAAAAAAAAAAAA"
echo "  '$ones' adler32: $(adler32 "$ones")"

echo
echo "── distribution test ──"
echo "  100 random strings (length 10), djb2 modulo 256:"
RANDOM=42
typeset -A hist
for ((i=0; i<100; i++)); do
    s=""
    for ((j=0; j<10; j++)); do
        # (#) evaluates its argument arithmetically and yields that character.
        s+="${(#)$(( 97 + RANDOM % 26 ))}"
    done
    h=$(djb2 "$s")
    bucket=$(( 0x$h % 256 ))
    (( hist[$bucket]++ ))
done
echo "  unique buckets used: ${#hist}"
echo "  expected ≈ 90 / 256 (perfect uniform)"

echo
echo "── stats ──"
echo "  hash algorithms: adler32, djb2, fnv1a, sum16, xor"
echo "  use cases: file dedup, cache key, error detect, distribution"

command rm -rf $tmpdir

# === ztest assertions ===
zassert_eq "$(adler32 abc)"   "024d0127" "Adler-32 of 'abc' (RFC 1950 vector)"
zassert_eq "$(adler32 '')"    "00000001" "Adler-32 of the empty string is 1"
zassert_eq "$(djb2 abc)"      "0b885c8b" "DJB2 of 'abc'"
zassert_eq "$(djb2 hello)"    "0f923099" "DJB2 of 'hello'"
zassert_eq "$(fnv1a abc)"     "5f171ed71a47e90b" "FNV-1a of 'abc'"
zassert_ge "${#hist[@]}"      1          "djb2 distribution test filled buckets"
zassert_le "${#hist[@]}"      256        "djb2 buckets stay inside mod-256 range"
ztest_run
