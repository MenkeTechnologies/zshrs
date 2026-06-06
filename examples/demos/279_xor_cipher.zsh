#!/usr/bin/env zshrs
# XOR cipher — keystream + hex display + frequency analysis.

# XOR each char of $1 with key, return hex bytes.
xor_hex() {
    local s=$1 key=$2 out=""
    local i sc kc x kidx kch sch
    local klen=${#key}
    for ((i=1; i<=${#s}; i++)); do
        sch="${s[i]}"
        sc=$(( #sch ))
        kidx=$(( (i-1) % klen + 1 ))
        kch="${key[kidx]}"
        kc=$(( #kch ))
        x=$(( sc ^ kc ))
        out+=$(printf "%02x " $x)
    done
    echo "${out% }"
}

# Decode hex bytes (space-sep) to chars with key.
xor_decode() {
    local hex=$1 key=$2 out=""
    local -a bytes
    bytes=( ${=hex} )
    local i b kc x kidx kch
    local klen=${#key}
    for ((i=1; i<=${#bytes}; i++)); do
        b=$(( 0x${bytes[i]} ))
        kidx=$(( (i-1) % klen + 1 ))
        kch="${key[kidx]}"
        kc=$(( #kch ))
        x=$(( b ^ kc ))
        out+=$(printf "\\$(printf %03o $x)")
    done
    echo "$out"
}

echo "── round-trip XOR ──"
pairs=(
    "hello~key"
    "The quick brown fox~secret"
    "ZSHRS~XOR"
    "1234567890~9"
    "A~A"
    "Abc!@#~password"
)
for p in "${pairs[@]}"; do
    plain="${p%~*}"
    key="${p#*~}"
    enc=$(xor_hex "$plain" "$key")
    dec=$(xor_decode "$enc" "$key")
    printf "  plain:  %s\n  key:    %s\n  hex:    %s\n  dec:    %s   %s\n\n" \
        "$plain" "$key" "$enc" "$dec" "$([[ $dec == $plain ]] && echo ✓ || echo ✗)"
done

echo "── self-XOR is identity ──"
test_str="zshrs is the future"
once=$(xor_hex "$test_str" "K")
back=$(xor_decode "$once" "K")
echo "  '$test_str' xor K xor K = '$back'   $([[ $back == $test_str ]] && echo ✓ || echo ✗)"

echo
echo "── repeating-key XOR vulnerability: byte frequency ──"
# Build long ciphertext from repeating key 'X'.
plaintext="the rain in spain stays mainly in the plain the rain in spain stays mainly in the plain"
ciphertext_hex=$(xor_hex "$plaintext" "X")
ciphertext_bytes=( ${=ciphertext_hex} )

typeset -A byte_freq
for b in "${ciphertext_bytes[@]}"; do
    (( byte_freq[$b]++ ))
done

echo "  ciphertext byte frequency (top 5):"
sorted_bytes=("${(@k)byte_freq}")
# Bubble sort by freq desc.
n=${#sorted_bytes}
for ((i=1; i<=n; i++)); do
    for ((j=i+1; j<=n; j++)); do
        if (( byte_freq[${sorted_bytes[i]}] < byte_freq[${sorted_bytes[j]}] )); then
            tmp=${sorted_bytes[i]}
            sorted_bytes[i]=${sorted_bytes[j]}
            sorted_bytes[j]=$tmp
        fi
    done
done
top=0
for b in "${sorted_bytes[@]}"; do
    if (( top < 5 )); then
        cnt=${byte_freq[$b]}
        # Decode this byte assuming key is 'X' (0x58).
        plain_byte=$(( 0x$b ^ 0x58 ))
        plain_ch=$(printf "\\$(printf %03o $plain_byte)")
        printf "    0x%s × %2d  → plain '%s'\n" "$b" $cnt "$plain_ch"
        (( top++ ))
    fi
done

echo
echo "── byte-by-byte hex dump (first 32 bytes) ──"
short_plain="zshrs is a compiled shell"
hex=$(xor_hex "$short_plain" "key")
bytes=( ${=hex} )
for ((i=1; i<=${#short_plain}; i++)); do
    if (( i > 32 )); then break; fi
    ch="${short_plain[i]}"
    ord=$(( #ch ))
    printf "  '%s' (0x%02x)  →  0x%s\n" "$ch" $ord "${bytes[i]}"
done

echo
echo "── multi-key length probe (Hamming-style) ──"
# Just count bit differences between consecutive bytes in cipher.
hamming() {
    local a=$1 b=$2
    local x=$(( a ^ b ))
    local n=0
    while (( x > 0 )); do
        (( n += x & 1 ))
        (( x >>= 1 ))
    done
    echo $n
}

for klen in 1 2 3 4 5; do
    total_dist=0
    pair_count=0
    for ((i=1; i+klen<=${#bytes}; i++)); do
        a=$(( 0x${bytes[i]} ))
        b=$(( 0x${bytes[i+klen]} ))
        d=$(hamming $a $b)
        (( total_dist += d ))
        (( pair_count++ ))
    done
    if (( pair_count > 0 )); then
        normalized=$(( total_dist * 100 / pair_count / klen ))
        printf "  keylen=%d : avg Hamming/bit ≈ %d.%02d\n" $klen $((normalized/100)) $((normalized%100))
    fi
done

# === ztest assertions ===
# Round-trip is the defining property
zassert_eq "$(xor_decode "$(xor_hex "hello" "key")" "key")" "hello" "round-trip 'hello' / key='key'"
zassert_eq "$(xor_decode "$(xor_hex "ZSHRS" "XOR")" "XOR")" "ZSHRS" "round-trip 'ZSHRS'"
zassert_eq "$(xor_decode "$(xor_hex "1234567890" "9")" "9")" "1234567890" "round-trip with single-char key"
# Known hex output for short string + key
zassert_eq "$(xor_hex "hello" "key")" "03 00 15 07 0a" "xor_hex(hello,key) bytes"
zassert_eq "$(xor_hex "A" "A")" "00" "A xor A = 00"
# Hamming popcount
zassert_eq "$(hamming 0 0)"    0  "hamming(0,0) = 0"
zassert_eq "$(hamming 0 255)"  8  "hamming(0,255) = 8 bits"
zassert_eq "$(hamming 1 2)"    2  "hamming(1,2) = 2 (0b01 vs 0b10)"
zassert_eq "$(hamming 7 0)"    3  "hamming(7,0) = 3 set bits"
# Self-XOR identity
zassert_eq "$back" "$test_str" "(plain xor K) xor K = plain"
ztest_run
