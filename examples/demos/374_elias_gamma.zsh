#!/usr/bin/env zshrs
# Elias gamma codes — universal codes for positive integers.
#
# Encoding of n ≥ 1:
#   1. Let k = floor(log2(n)).
#   2. Write k zeros, then n in k+1 bits.
#
# Example: n=5 → k=2 → "00" + "101" = "00101".
#
# Used in compression schemes where the distribution of integers is
# unknown a priori but smaller values dominate (e.g. inverted indexes,
# audio coding). Decoder reads leading zeros to determine k, then
# reads k+1 more bits.
#
# Also includes Elias **delta** (gamma-encode k+1, then write n's
# lower-order bits) — more efficient asymptotically.

# n → bit-string (e.g. "00101")
gamma_encode() {
    local n=$1
    if (( n < 1 )); then
        echo "ERR"
        return 1
    fi
    # Compute k = floor(log2 n).
    local k=0 v=$n
    while (( v > 1 )); do
        (( v >>= 1 ))
        (( k++ ))
    done
    # Binary representation of n in k+1 bits.
    local bits=""
    local i
    for ((i=k; i>=0; i--)); do
        bits+=$(( (n >> i) & 1 ))
    done
    local zeros=""
    for ((i=0; i<k; i++)); do zeros+="0"; done
    print -rn -- "${zeros}${bits}"
}

# Decode bit-string at position $2 (1-based). Echoes "value newpos".
gamma_decode_at() {
    local s=$1 pos=$2
    local n=${#s}
    # Count leading zeros.
    local k=0
    while (( pos + k <= n )) && [[ ${s[pos+k]} == "0" ]]; do
        (( k++ ))
    done
    if (( pos + k > n )); then
        echo "ERR-truncated"
        return 1
    fi
    # Read k+1 bits starting at pos+k.
    local v=0 i b
    for ((i=0; i<=k; i++)); do
        b=${s[pos+k+i]}
        v=$(( v*2 + b ))
    done
    echo "$v $(( pos + 2*k + 1 ))"
}

# Decode entire bit-string as sequence of gamma codes.
gamma_decode_seq() {
    local s=$1
    local pos=1
    local -a out
    while (( pos <= ${#s} )); do
        local pair=$(gamma_decode_at "$s" $pos)
        local val=${pair% *}
        local newpos=${pair#* }
        out+=("$val")
        pos=$newpos
    done
    echo "${out[@]}"
}

# Elias delta: gamma-code (k+1) followed by n's low k bits.
delta_encode() {
    local n=$1
    if (( n < 1 )); then
        echo "ERR"
        return 1
    fi
    local k=0 v=$n
    while (( v > 1 )); do
        (( v >>= 1 ))
        (( k++ ))
    done
    local prefix=$(gamma_encode $(( k + 1 )))
    # Low k bits of n (not including leading 1).
    local low=""
    local i
    for ((i=k-1; i>=0; i--)); do
        low+=$(( (n >> i) & 1 ))
    done
    print -rn -- "${prefix}${low}"
}

echo "=== Elias gamma table ==="
printf "%4s | %-12s | %s\n" "n" "gamma" "delta"
printf "%4s-+-%-12s-+-%s\n" "----" "------------" "---------"
for n in 1 2 3 4 5 6 7 8 15 16 32 100 1000; do
    g=$(gamma_encode $n)
    d=$(delta_encode $n)
    printf "%4d | %-12s | %s\n" $n $g $d
done

echo
echo "=== sequence round-trip ==="
seq="1 2 3 5 8 13 21 34 55 89"
echo "input:    $seq"
encoded=""
for n in ${(z)seq}; do
    encoded+=$(gamma_encode $n)
done
echo "bitstream (${#encoded} bits): $encoded"
decoded=$(gamma_decode_seq "$encoded")
echo "decoded:  $decoded"

# === ztest ===
zassert_eq "$(gamma_encode 1)"  "1"        "γ(1) = 1"
zassert_eq "$(gamma_encode 2)"  "010"      "γ(2) = 010"
zassert_eq "$(gamma_encode 3)"  "011"      "γ(3) = 011"
zassert_eq "$(gamma_encode 4)"  "00100"    "γ(4) = 00100"
zassert_eq "$(gamma_encode 5)"  "00101"    "γ(5) = 00101"
zassert_eq "$(gamma_encode 8)"  "0001000"  "γ(8) = 0001000"
zassert_eq "$(gamma_encode 15)" "0001111"  "γ(15) = 0001111"
zassert_eq "$(gamma_encode 16)" "000010000" "γ(16) = 000010000"

zassert_eq "$(delta_encode 1)"  "1"        "δ(1) = 1"
zassert_eq "$(delta_encode 2)"  "0100"     "δ(2) = 0100"
zassert_eq "$(delta_encode 5)"  "01101"    "δ(5) = 01101"

# Round-trip every n in 1..50.
ok_count=0
for n in {1..50}; do
    e=$(gamma_encode $n)
    pair=$(gamma_decode_at "$e" 1)
    val=${pair% *}
    if [[ $val == $n ]]; then
        (( ok_count++ ))
    fi
done
zassert_eq "$ok_count" "50" "gamma round-trip for n ∈ [1,50]"

# Sequence round-trip.
zassert_eq "$decoded" "1 2 3 5 8 13 21 34 55 89" "Fibonacci-ish seq round-trip"

ztest_run
