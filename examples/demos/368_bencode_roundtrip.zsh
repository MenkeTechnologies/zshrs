#!/usr/bin/env zshrs
# Bencode encoder/decoder — the BitTorrent wire format.
#
# Bencode grammar (BEP-3):
#   integer  ::=  'i' DIGIT+ 'e'         (negatives allowed; no leading zeros)
#   string   ::=  LEN ':' BYTES          (length-prefixed, ASCII only here)
#   list     ::=  'l' VALUE* 'e'
#   dict     ::=  'd' (string VALUE)* 'e' (keys sorted lexicographically)
#
# This demo encodes/decodes round-trip and pins behavior with assertions.

# ──────── ENCODER ────────
bencode_int() { print -rn -- "i${1}e"; }

bencode_str() {
    local s=$1
    print -rn -- "${#s}:${s}"
}

# bencode_list val1 val2 val3 ... (each already-bencoded)
bencode_list() {
    local out="l" v
    for v in "$@"; do out+=$v; done
    out+="e"
    print -rn -- "$out"
}

# bencode_dict "key1" "val1-bencoded" "key2" "val2-bencoded" ...
# Caller is responsible for sorted keys; we verify here.
bencode_dict() {
    local -a keys vals
    local i
    for ((i=1; i<=$#; i+=2)); do
        keys+=("${@[i]}")
        vals+=("${@[i+1]}")
    done
    # Sort by key (stable parallel sort).
    local -a order
    for ((i=1; i<=${#keys}; i++)); do order+=($i); done
    local a b tmp swap=1
    while (( swap )); do
        swap=0
        for ((i=1; i<${#order}; i++)); do
            a=${order[i]}
            b=${order[i+1]}
            if [[ ${keys[a]} > ${keys[b]} ]]; then
                tmp=${order[i]}; order[i]=${order[i+1]}; order[i+1]=$tmp
                swap=1
            fi
        done
    done
    local out="d" idx
    for idx in "${order[@]}"; do
        out+=$(bencode_str "${keys[idx]}")
        out+="${vals[idx]}"
    done
    out+="e"
    print -rn -- "$out"
}

# ──────── DECODER ────────
# Decoder is stateful: BD_POS tracks current byte offset 1-based into BD_BUF.
# Output is written to BD_OUT — a flat trace string for inspection.
typeset -g BD_BUF=""
typeset -gi BD_POS=1
typeset -g BD_OUT=""

bd_peek() { print -rn -- "${BD_BUF[BD_POS]}"; }

bd_int() {
    # Caller already consumed 'i'.
    local neg="" n=""
    if [[ ${BD_BUF[BD_POS]} == "-" ]]; then
        neg="-"
        (( BD_POS++ ))
    fi
    while [[ ${BD_BUF[BD_POS]} == [0-9] ]]; do
        n+=${BD_BUF[BD_POS]}
        (( BD_POS++ ))
    done
    # Must end with 'e'.
    if [[ ${BD_BUF[BD_POS]} != "e" ]]; then
        BD_OUT+="ERR:int-no-e@${BD_POS} "
        return 1
    fi
    (( BD_POS++ ))
    BD_OUT+="INT(${neg}${n}) "
}

bd_str() {
    local len="" c
    while [[ ${BD_BUF[BD_POS]} == [0-9] ]]; do
        len+=${BD_BUF[BD_POS]}
        (( BD_POS++ ))
    done
    if [[ ${BD_BUF[BD_POS]} != ":" ]]; then
        BD_OUT+="ERR:str-no-colon@${BD_POS} "
        return 1
    fi
    (( BD_POS++ ))
    local s=""
    local i
    for ((i=0; i<len; i++)); do
        s+=${BD_BUF[BD_POS+i]}
    done
    (( BD_POS += len ))
    BD_OUT+="STR(${s}) "
}

bd_value() {
    local c=$(bd_peek)
    case "$c" in
        i)  (( BD_POS++ )); bd_int ;;
        l)  (( BD_POS++ ))
            BD_OUT+="LIST{ "
            while [[ $(bd_peek) != "e" ]]; do bd_value || return 1; done
            (( BD_POS++ ))
            BD_OUT+="} " ;;
        d)  (( BD_POS++ ))
            BD_OUT+="DICT{ "
            while [[ $(bd_peek) != "e" ]]; do
                bd_str || return 1
                bd_value || return 1
            done
            (( BD_POS++ ))
            BD_OUT+="} " ;;
        [0-9]) bd_str ;;
        *)  BD_OUT+="ERR:unknown(${c})@${BD_POS} "
            return 1 ;;
    esac
}

bdecode() {
    BD_BUF=$1
    BD_POS=1
    BD_OUT=""
    bd_value
    print -rn -- "${BD_OUT}"
}

# ──────── DEMO ────────
echo "=== bencode primitives ==="
echo "int 42        → $(bencode_int 42)"
echo "int -7        → $(bencode_int -7)"
echo "str 'spam'    → $(bencode_str spam)"
echo "list [42,'x'] → $(bencode_list "$(bencode_int 42)" "$(bencode_str x)")"
echo "dict {a:1,b:'q'} → $(bencode_dict a "$(bencode_int 1)" b "$(bencode_str q)")"

echo
echo "=== nested torrent-info-style structure ==="
nested=$(bencode_dict \
    announce "$(bencode_str http://tracker.invalid/announce)" \
    info "$(bencode_dict \
        length "$(bencode_int 1024)" \
        name   "$(bencode_str hello.bin)" \
        pieces "$(bencode_str ABCD)")")
echo "$nested"

echo
echo "=== decode trace ==="
bdecode "$nested"
echo

echo
echo "=== round-trip dict order pin ==="
# Insert keys out-of-order — encoder must sort them.
out=$(bencode_dict z "$(bencode_int 3)" a "$(bencode_int 1)" m "$(bencode_int 2)")
echo "encoded: $out"

# === ztest ===
zassert_eq "$(bencode_int 0)"           "i0e"        "encode 0"
zassert_eq "$(bencode_int 42)"          "i42e"       "encode 42"
zassert_eq "$(bencode_int -1)"          "i-1e"       "encode -1"
zassert_eq "$(bencode_str spam)"        "4:spam"     "encode 'spam'"
zassert_eq "$(bencode_str '')"          "0:"         "encode empty string"
zassert_eq "$(bencode_list "$(bencode_int 1)" "$(bencode_int 2)")" \
                                        "li1ei2ee"   "encode list [1,2]"
zassert_eq "$out" "d1:ai1e1:mi2e1:zi3ee" "dict sorted lex"
zassert_contains "$(bdecode "$nested")" "STR(http://tracker.invalid/announce)" \
                                        "decoder sees announce URL"
zassert_contains "$(bdecode "$nested")" "INT(1024)" "decoder sees length"
zassert_contains "$(bdecode "$nested")" "STR(hello.bin)" "decoder sees name"
ztest_run
