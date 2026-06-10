#!/usr/bin/env zshrs
# Hamming(7,4) — single-bit error-correcting block code.
#
# Encodes 4 data bits into 7 transmitted bits by adding 3 parity bits.
# Bit positions (1-based): 1=p1, 2=p2, 3=d1, 4=p3, 5=d2, 6=d3, 7=d4
#   p1 covers positions 1,3,5,7  (bit-mask 001)
#   p2 covers positions 2,3,6,7  (bit-mask 010)
#   p3 covers positions 4,5,6,7  (bit-mask 100)
#
# Any single-bit flip in the 7-bit word produces a non-zero syndrome
# whose binary value IS the flipped bit's position — that's the
# elegant Hamming trick. Two-bit errors are not corrected (would
# require SECDED with an extra parity bit).

# Encode 4 data bits → echoes 7 transmitted bits space-separated.
hamming_encode() {
    local d1=$1 d2=$2 d3=$3 d4=$4
    local p1=$(( (d1 + d2 + d4) % 2 ))
    local p2=$(( (d1 + d3 + d4) % 2 ))
    local p3=$(( (d2 + d3 + d4) % 2 ))
    echo "$p1 $p2 $d1 $p3 $d2 $d3 $d4"
}

# Compute syndrome (0–7) for a 7-bit word.
hamming_syndrome() {
    local -a b
    b=("$@")
    local s1=$(( (b[1] + b[3] + b[5] + b[7]) % 2 ))
    local s2=$(( (b[2] + b[3] + b[6] + b[7]) % 2 ))
    local s3=$(( (b[4] + b[5] + b[6] + b[7]) % 2 ))
    echo $(( s1 + s2*2 + s3*4 ))
}

# Correct (if needed) and extract 4 data bits.
hamming_correct() {
    local -a b
    b=("$@")
    local syndrome=$(hamming_syndrome "${b[@]}")
    if (( syndrome != 0 )); then
        b[syndrome]=$(( 1 - b[syndrome] ))
    fi
    echo "${b[3]} ${b[5]} ${b[6]} ${b[7]}"
}

# Round-trip with corruption at every bit position.
demo_position() {
    local data="$1"
    local d1=${data[1]} d2=${data[3]} d3=${data[5]} d4=${data[7]}
    local enc=$(hamming_encode $d1 $d2 $d3 $d4)
    echo "data ${data} → encoded ${enc}"
    local -a e
    e=(${(z)enc})
    local pos
    for pos in 1 2 3 4 5 6 7; do
        local -a corrupt
        corrupt=("${e[@]}")
        corrupt[pos]=$(( 1 - corrupt[pos] ))
        local syn=$(hamming_syndrome "${corrupt[@]}")
        local recov=$(hamming_correct "${corrupt[@]}")
        echo "  flip pos $pos → corrupt='${(j: :)corrupt}' syndrome=$syn recovered='$recov'"
    done
}

echo "=== Hamming(7,4) demo ==="
demo_position "1 0 1 1"
echo
demo_position "0 1 1 0"

# === ztest ===
zassert_eq "$(hamming_encode 1 0 1 1)" "0 1 1 0 0 1 1" "encode 1011"
zassert_eq "$(hamming_encode 0 0 0 0)" "0 0 0 0 0 0 0" "encode 0000"
zassert_eq "$(hamming_encode 1 1 1 1)" "1 1 1 1 1 1 1" "encode 1111"
zassert_eq "$(hamming_encode 1 0 0 0)" "1 1 1 0 0 0 0" "encode 1000"

# Clean codeword → syndrome 0.
zassert_eq "$(hamming_syndrome 0 1 1 0 0 1 1)" "0" "clean codeword syndrome 0"

# Each single-bit flip → correction recovers original 1011.
for pos in 1 2 3 4 5 6 7; do
    word=(0 1 1 0 0 1 1)
    word[pos]=$(( 1 - word[pos] ))
    syn=$(hamming_syndrome "${word[@]}")
    zassert_eq "$syn" "$pos" "syndrome equals flipped position $pos"
    recov=$(hamming_correct "${word[@]}")
    zassert_eq "$recov" "1 0 1 1" "correct single flip at pos $pos"
done

ztest_run
