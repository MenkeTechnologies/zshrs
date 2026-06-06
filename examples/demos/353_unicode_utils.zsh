#!/usr/bin/env zshrs
# Unicode-aware utilities — width detection, normalization, escape.

# Is char ASCII?
is_ascii() {
    local c=$1
    local ord=$(( #c ))
    (( ord >= 0 && ord <= 127 ))
}

# Is char a control char (0-31, 127)?
is_control() {
    local c=$1
    local ord=$(( #c ))
    (( ord < 32 || ord == 127 ))
}

# Display width estimate (1 for most, 0 for combining, 2 for CJK).
# Note: this is a rough heuristic for terminal display.
char_width() {
    local c=$1
    local ord=$(( #c ))
    # Control: 0
    (( ord < 32 )) && { echo 0; return; }
    (( ord == 127 )) && { echo 0; return; }
    # Combining (rough range): 0x300-0x36F
    (( ord >= 768 && ord <= 879 )) && { echo 0; return; }
    # CJK Unified Ideographs: 0x4E00-0x9FFF (wide)
    (( ord >= 19968 && ord <= 40959 )) && { echo 2; return; }
    # Hiragana 0x3040-0x309F + Katakana 0x30A0-0x30FF (wide)
    (( ord >= 12352 && ord <= 12543 )) && { echo 2; return; }
    # Hangul (Korean) 0xAC00-0xD7AF (wide)
    (( ord >= 44032 && ord <= 55215 )) && { echo 2; return; }
    # Emoji rough range (Misc Symbols + Pictographs)
    (( ord >= 128512 && ord <= 128591 )) && { echo 2; return; }
    # Default: 1
    echo 1
}

# String display width.
display_width() {
    local s=$1 total=0 i c w
    for ((i=1; i<=${#s}; i++)); do
        c="${s[i]}"
        w=$(char_width "$c")
        (( total += w ))
    done
    echo $total
}

# Hex escape: \xNN.
hex_escape() {
    local s=$1 out="" i c ord
    for ((i=1; i<=${#s}; i++)); do
        c="${s[i]}"
        ord=$(( #c ))
        if (( ord < 128 && ord >= 32 && ord != 92 && ord != 34 )); then
            out+="$c"
        else
            out+=$(printf '\\x%02x' $ord)
        fi
    done
    echo "$out"
}

# JSON-style \u escape for non-ASCII.
json_escape() {
    local s=$1 out="" i c ord
    for ((i=1; i<=${#s}; i++)); do
        c="${s[i]}"
        ord=$(( #c ))
        if (( ord < 128 && ord >= 32 && ord != 92 && ord != 34 )); then
            case $c in
                '\') out+='\\\\' ;;
                '"') out+='\"' ;;
                *) out+="$c" ;;
            esac
        elif (( ord < 32 )); then
            case $ord in
                8) out+='\b' ;;
                9) out+='\t' ;;
                10) out+='\n' ;;
                12) out+='\f' ;;
                13) out+='\r' ;;
                *) out+=$(printf '\\u%04x' $ord) ;;
            esac
        else
            out+=$(printf '\\u%04x' $ord)
        fi
    done
    echo "$out"
}

# Convert hex back to char.
hex_unescape() {
    local s=$1 out="" i n j
    n=${#s}
    i=1
    while (( i <= n )); do
        local c="${s[i]}"
        if [[ $c == '\' && ${s[i+1]} == 'x' ]] && (( i + 3 <= n )); then
            local hex="${s[i+2,i+3]}"
            local ord=$(( 0x$hex ))
            out+=$(printf "\\$(printf %03o $ord)")
            (( i += 4 ))
        else
            out+="$c"
            (( i++ ))
        fi
    done
    echo "$out"
}

echo "── character classifications ──"
samples=(
    "A:ASCII letter"
    "0:ASCII digit"
    "!:ASCII punct"
    " :ASCII space"
    "$'\t':TAB control"
    "α:Greek letter"
    "中:CJK ideograph"
    "あ:Hiragana"
)
for sample in "${samples[@]}"; do
    c="${sample%:*}"
    desc="${sample#*:}"
    local ord=$(( #c ))
    local w=$(char_width "$c")
    local ascii_y="✗"
    is_ascii "$c" && ascii_y="✓"
    local ctrl_y="✗"
    is_control "$c" && ctrl_y="✓"
    printf "  '%s' (ord %5d, %-25s) width=%d ascii=%s ctrl=%s\n" \
        "$c" $ord "$desc" $w "$ascii_y" "$ctrl_y"
done

echo
echo "── display width ──"
strings=(
    "hello"
    "héllo"
    "café"
    "中国"
    "abc中"
    "あいうえお"
    "Δfoo"
    "🚀 rocket"
    "naive: hello + tab"
)
for s in "${strings[@]}"; do
    w=$(display_width "$s")
    char_count=${#s}
    printf "  '%-25s' chars=%2d  display_width=%d\n" "$s" $char_count $w
done

echo
echo "── hex escape (for binary-safe transmission) ──"
samples=(
    "hello world"
    "tab here: $'\t' end"
    "non-ascii: café"
    "null byte: $'\x00' end"
    "all printable"
)
for s in "${samples[@]}"; do
    e=$(hex_escape "$s")
    printf "  in:  %s\n  hex: %s\n\n" "$s" "$e"
done

echo "── JSON escape ──"
for s in \
    'simple text' \
    '"quoted"' \
    'back\slash' \
    "newline$'\n'embedded" \
    'café' \
    'unicode 中文 here'; do
    j=$(json_escape "$s")
    printf "  in:   %s\n  json: %s\n\n" "$s" "$j"
done

echo "── byte vs char count ──"
multi_byte=(
    "hello"
    "héllo"
    "中国"
    "🚀🎉"
)
for s in "${multi_byte[@]}"; do
    char_count=${#s}
    # UTF-8 byte estimate: ASCII=1, Latin1=2, BMP=3, supplementary=4
    bytes=0
    for ((i=1; i<=${#s}; i++)); do
        c="${s[i]}"
        ord=$(( #c ))
        if (( ord < 128 )); then
            (( bytes += 1 ))
        elif (( ord < 2048 )); then
            (( bytes += 2 ))
        elif (( ord < 65536 )); then
            (( bytes += 3 ))
        else
            (( bytes += 4 ))
        fi
    done
    printf "  '%-12s' chars=%d  UTF-8 bytes≈%d\n" "$s" $char_count $bytes
done

echo
echo "── Unicode notes ──"
echo "  zsh internally stores strings as wide chars when ZSH_NO_UTF8 is unset"
echo "  \${#s} counts code points (chars), not bytes"
echo "  display width depends on terminal — use wcwidth() for accuracy"
echo "  combining marks (U+0300..036F) have width 0"
echo "  CJK + emoji are typically width 2 (wide cells)"

# === ztest assertions ===
is_ascii "A" && a=1 || a=0
zassert_eq "$a" "1"             "A is ascii"
is_ascii "中" && a=1 || a=0
zassert_eq "$a" "0"             "中 is not ascii"
is_control "A" && a=1 || a=0
zassert_eq "$a" "0"             "A is not control"
zassert_eq "$(char_width A)"  "1"  "char_width ASCII = 1"
zassert_eq "$(char_width 中)" "2"  "char_width CJK = 2"
zassert_eq "$(char_width あ)" "2"  "char_width Hiragana = 2"
zassert_eq "$(display_width hello)"  "5"  "display_width hello"
zassert_eq "$(display_width 中国)"   "4"  "display_width 中国 = 4 (2+2)"
zassert_eq "$(display_width abc中)"  "5"  "display_width abc中 = 5 (1+1+1+2)"
ztest_run
