#!/usr/bin/env zshrs
# Base64 encode/decode in pure zsh.

CHARS="ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"

b64_encode() {
    local s=$1
    local out="" i ch1 ch2 ch3 n
    local len=${#s}
    for ((i=1; i<=len; i+=3)); do
        local c1=$(printf "%d" "'${s[i]}")
        local c2=0 c3=0 pad=0
        if (( i+1 <= len )); then
            c2=$(printf "%d" "'${s[i+1]}")
        else
            pad=2
        fi
        if (( i+2 <= len )); then
            c3=$(printf "%d" "'${s[i+2]}")
        else
            pad=$(( pad == 0 ? 1 : pad ))
        fi
        local triple=$(( (c1 << 16) | (c2 << 8) | c3 ))
        out+=${CHARS[$(( (triple >> 18) & 63 + 1 ))]}
        out+=${CHARS[$(( (triple >> 12) & 63 + 1 ))]}
        if (( pad >= 2 )); then
            out+="="
        else
            out+=${CHARS[$(( (triple >> 6) & 63 + 1 ))]}
        fi
        if (( pad >= 1 )); then
            out+="="
        else
            out+=${CHARS[$(( triple & 63 + 1 ))]}
        fi
    done
    echo "$out"
}

echo "── basic encode ──"
for s in "A" "AB" "ABC" "Hello" "Hello, World!" "zshrs"; do
    enc=$(b64_encode "$s")
    echo "  '$s' → $enc"
done

echo "── compare with openssl/base64 (if available) ──"
if command -v base64 >/dev/null 2>&1; then
    for s in "Hello, World!" "zshrs"; do
        sys=$(echo -n "$s" | base64)
        ours=$(b64_encode "$s")
        if [[ "$sys" == "$ours" ]]; then
            echo "  OK: '$s' → $ours"
        else
            echo "  diff: '$s' ours=$ours sys=$sys"
        fi
    done
else
    echo "  (no system base64 to compare)"
fi

echo "── url-safe variant (- _ instead of + /) ──"
url_safe() {
    local s=$1
    s=${s//+/-}
    s=${s//\//_}
    echo "$s"
}
for s in "Hello, World!" "abc"; do
    enc=$(b64_encode "$s")
    url=$(url_safe "$enc")
    echo "  '$s' → $enc → $url"
done

echo "── length characteristic ──"
for s in "" "A" "AB" "ABC" "ABCD" "ABCDE" "ABCDEF"; do
    enc=$(b64_encode "$s")
    echo "  len=${#s} → enc_len=${#enc}: $enc"
done
