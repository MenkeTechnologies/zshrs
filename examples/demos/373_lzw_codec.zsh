#!/usr/bin/env zshrs
# LZW (Lempel–Ziv–Welch) compression — the algorithm behind GIF and the
# original Unix `compress` utility.
#
# Encoder maintains a dictionary mapping strings → integer codes. It
# starts with 256 single-byte entries, then greedily emits the code
# for the longest prefix found in the dictionary and inserts
# prefix+next char as a new entry.
#
# Decoder mirrors: rebuilds the same dictionary on-the-fly without
# transmission. The "kwkwk" edge case (decoder needs an entry just
# being defined) is handled by detecting code == next_code.
#
# Dictionary keys are **ord-encoded byte sequences** ("65" for 'A',
# "65 66" for "AB") so the assoc-array index doesn't trip on shell
# metacharacters like '[', ']', '$', '\'.

typeset -gA LZW_DICT       # "ord ord ord" → code
typeset -gA LZW_REVDICT    # code           → "ord ord ord"
typeset -gi LZW_NEXT=0

# Char → ord (1-byte ASCII; shell takes care of UTF-8 multi-byte
# only if we operate at byte level, which we don't here for demo
# legibility).
ord() { printf '%d' "'$1"; }

# "ord-seq" → original string.
ord_seq_to_str() {
    local seq=$1
    local -a parts
    parts=(${(z)seq})
    local out="" p
    for p in "${parts[@]}"; do
        out+=$(printf "\\$(printf %03o $p)")
    done
    print -rn -- "$out"
}

lzw_init_alpha() {
    LZW_DICT=()
    LZW_REVDICT=()
    LZW_NEXT=0
    local i
    for ((i=0; i<256; i++)); do
        LZW_DICT[$i]=$LZW_NEXT
        LZW_REVDICT[$LZW_NEXT]=$i
        (( LZW_NEXT++ ))
    done
}

# Encode → echoes space-separated code stream.
lzw_encode() {
    local s=$1
    lzw_init_alpha
    local w_ords="" c_ord
    local i n=${#s} out=""
    for ((i=1; i<=n; i++)); do
        c_ord=$(ord "${s[i]}")
        local wc_ords
        if [[ -z $w_ords ]]; then
            wc_ords=$c_ord
        else
            wc_ords="${w_ords} ${c_ord}"
        fi
        if [[ -n ${LZW_DICT[$wc_ords]+x} ]]; then
            w_ords=$wc_ords
        else
            out+="${LZW_DICT[$w_ords]} "
            LZW_DICT[$wc_ords]=$LZW_NEXT
            LZW_REVDICT[$LZW_NEXT]=$wc_ords
            (( LZW_NEXT++ ))
            w_ords=$c_ord
        fi
    done
    if [[ -n $w_ords ]]; then
        out+="${LZW_DICT[$w_ords]}"
    fi
    print -rn -- "${out% }"
}

# Decode space-separated code stream → original string.
lzw_decode() {
    local codes_str=$1
    lzw_init_alpha
    local -a codes
    codes=(${(z)codes_str})
    if (( ${#codes} == 0 )); then
        return
    fi
    local prev=${LZW_REVDICT[${codes[1]}]}
    local out_ords=$prev
    local i code entry
    for ((i=2; i<=${#codes}; i++)); do
        code=${codes[i]}
        if [[ -n ${LZW_REVDICT[$code]+x} ]]; then
            entry=${LZW_REVDICT[$code]}
        elif (( code == LZW_NEXT )); then
            # Self-reference: entry being defined this step is prev+prev[0].
            local first_ord=${prev%% *}
            entry="${prev} ${first_ord}"
        else
            print -rn -- "ERR:bad-code(${code})"
            return 1
        fi
        out_ords+=" ${entry}"
        local first_of_entry=${entry%% *}
        LZW_REVDICT[$LZW_NEXT]="${prev} ${first_of_entry}"
        (( LZW_NEXT++ ))
        prev=$entry
    done
    ord_seq_to_str "$out_ords"
}

lzw_roundtrip() {
    local s=$1
    local enc=$(lzw_encode "$s")
    local dec=$(lzw_decode "$enc")
    local n_in=${#s}
    local -a codes
    codes=(${(z)enc})
    local n_codes=${#codes}
    echo "input  (${n_in} bytes): $s"
    echo "codes  (${n_codes} ints): $enc"
    echo "decoded         (match): $dec"
    [[ $s == $dec ]] && echo "✓ round-trip clean" || echo "✗ round-trip MISMATCH"
}

echo "=== LZW demo: TOBEORNOTTOBEORTOBEORNOT ==="
lzw_roundtrip "TOBEORNOTTOBEORTOBEORNOT"
echo
echo "=== LZW demo: ABABABABAB (high repetition) ==="
lzw_roundtrip "ABABABABAB"
echo
echo "=== LZW edge case: kwkwk (decoder self-resolve) ==="
lzw_roundtrip "kwkwk"

# === ztest ===
for s in \
    "TOBEORNOTTOBEORTOBEORNOT" \
    "ABABABABAB" \
    "kwkwk" \
    "A" \
    "AAAAAAAAAAAA" \
    "the quick brown fox jumps over the lazy dog" \
    "1234567890" \
; do
    enc=$(lzw_encode "$s")
    dec=$(lzw_decode "$enc")
    zassert_eq "$dec" "$s" "round-trip: '$s'"
done

red=$(lzw_encode "ABABABABABABABABABAB")
red_arr=(${(z)red})
zassert_lt "${#red_arr}" "20" "ABAB... × 10 compresses below input length"

zassert_eq "$(lzw_decode "$(lzw_encode "kwkwk")")" "kwkwk" "kwkwk edge round-trip"

ztest_run
