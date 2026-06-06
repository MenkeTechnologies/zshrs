#!/usr/bin/env zshrs
# One-Time Pad — generate random pad, encrypt, decrypt, verify.

# Generate pad of N bytes from $RANDOM.
gen_pad() {
    local n=$1 i hex=""
    for ((i=0; i<n; i++)); do
        hex+=$(printf "%02x " $(( RANDOM % 256 )))
    done
    echo "${hex% }"
}

# XOR encrypt msg using pad (in hex), output hex.
encrypt() {
    local msg=$1 pad=$2
    local -a pb
    pb=( ${=pad} )
    local i mc pc x out=""
    for ((i=1; i<=${#msg}; i++)); do
        local mch="${msg[i]}"
        mc=$(( #mch ))
        pc=$(( 0x${pb[i]} ))
        x=$(( mc ^ pc ))
        out+=$(printf "%02x " $x)
    done
    echo "${out% }"
}

# Decrypt: same XOR.
decrypt() {
    local ct=$1 pad=$2
    local -a cb pb
    cb=( ${=ct} )
    pb=( ${=pad} )
    local i c p x out=""
    for ((i=1; i<=${#cb}; i++)); do
        c=$(( 0x${cb[i]} ))
        p=$(( 0x${pb[i]} ))
        x=$(( c ^ p ))
        out+=$(printf "\\$(printf %03o $x)")
    done
    echo "$out"
}

echo "── round-trip with random pads ──"
RANDOM=42
messages=(
    "Attack at dawn"
    "Meet me at the bridge"
    "ZSHRS is the future"
    "The eagle has landed"
    "Hello, World!"
)
for msg in "${messages[@]}"; do
    pad=$(gen_pad ${#msg})
    ct=$(encrypt "$msg" "$pad")
    pt=$(decrypt "$ct" "$pad")
    ok="$([[ $pt == $msg ]] && echo ✓ || echo ✗)"
    printf "  msg:    %s\n  pad:    %s\n  ct:     %s\n  dec:    %s   %s\n\n" \
        "$msg" "$pad" "$ct" "$pt" "$ok"
done

echo "── pad reuse leaks XOR of plaintexts (key-reuse weakness) ──"
RANDOM=42
m1="HELLO WORLD"
m2="GREETINGS!!"
pad=$(gen_pad ${#m1})
c1=$(encrypt "$m1" "$pad")
c2=$(encrypt "$m2" "$pad")
# c1 XOR c2 = m1 XOR m2 (no pad).
echo "  m1: $m1"
echo "  m2: $m2"
echo "  c1: $c1"
echo "  c2: $c2"

# XOR c1 ⊕ c2.
b1=( ${=c1} )
b2=( ${=c2} )
xor_str=""
xor_ch=""
for ((i=1; i<=${#b1}; i++)); do
    x=$(( 0x${b1[i]} ^ 0x${b2[i]} ))
    xor_str+=$(printf "%02x " $x)
    if (( x >= 32 && x < 127 )); then
        xor_ch+=$(printf "\\$(printf %03o $x)")
    else
        xor_ch+="?"
    fi
done
echo "  c1⊕c2:    ${xor_str%% }"
echo "  printable: $xor_ch"

# Verify XOR matches plaintext XOR.
real_xor=""
for ((i=1; i<=${#m1}; i++)); do
    c1ch="${m1[i]}"
    c2ch="${m2[i]}"
    x=$(( #c1ch ^ #c2ch ))
    real_xor+=$(printf "%02x " $x)
done
echo "  m1⊕m2:    ${real_xor%% }"
echo "  match:    $([[ ${xor_str% } == ${real_xor% } ]] && echo ✓ || echo ✗)"

echo
echo "── perfectly random pad — Shannon entropy ──"
RANDOM=7
big_pad=$(gen_pad 256)
typeset -A pad_freq
for b in ${=big_pad}; do
    (( pad_freq[$b]++ ))
done

# Compute entropy: -sum(p * log2(p)).
# We'll estimate via histogram spread.
total=256
min=99999; max=0
for k in "${(@k)pad_freq}"; do
    c=${pad_freq[$k]}
    (( c < min )) && min=$c
    (( c > max )) && max=$c
done
unique=${#pad_freq}
echo "  pad size: $total bytes, $unique unique byte values"
echo "  min count: $min   max count: $max"
echo "  (uniform ≈ 1/byte; ${unique}/256 distinct seen)"

echo
echo "── pad-once never-reuse property ──"
m="zshrs forever"
pad1=$(gen_pad ${#m})
pad2=$(gen_pad ${#m})
c1=$(encrypt "$m" "$pad1")
c2=$(encrypt "$m" "$pad2")
echo "  same msg, two pads:"
echo "    ct1: $c1"
echo "    ct2: $c2"
[[ $c1 == $c2 ]] && echo "    ✗ ciphertexts match — pads identical (bad seed)"
[[ $c1 != $c2 ]] && echo "    ✓ ciphertexts differ — fresh pad each time"

# === ztest assertions ===
# OTP defining property: round-trip
m="zshrs forever"
pad=$(gen_pad ${#m})
zassert_eq "$(decrypt "$(encrypt "$m" "$pad")" "$pad")" "$m" "OTP round-trip"
# Single-byte pad
zassert_eq "$(decrypt "$(encrypt 'A' "ff")" "ff")" "A" "1-byte pad round-trip"
# encrypt produces 2 hex chars per input byte
ct=$(encrypt "hi" "00 00")
zassert_eq "$ct" "68 69" "encrypt with zero pad = original bytes (hex)"
ct=$(encrypt "A" "41")
zassert_eq "$ct" "00" "encrypt 'A' with pad 0x41 = 00 (self-xor)"
# Different pads produce different ciphertexts (high probability)
zassert_ne "$c1" "$c2" "different pads ⇒ different ciphertexts"
# Pad generation produces expected byte count
pad5=$(gen_pad 5)
words=( ${=pad5} )
zassert_eq "${#words}" 5 "gen_pad 5 returns 5 bytes"
# Key-reuse weakness verified: c1⊕c2 == m1⊕m2 stripped of pad
zassert_eq "${xor_str% }" "${real_xor% }" "c1⊕c2 = m1⊕m2 (key-reuse leak)"
# Entropy histogram has at least most bytes seen
zassert_ge "$unique" 100 "256-byte pad covers ≥100 distinct values"
ztest_run
