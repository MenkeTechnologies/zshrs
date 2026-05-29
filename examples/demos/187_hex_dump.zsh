#!/usr/bin/env zshrs
# Hex dump utility in pure zsh — od/xxd-like output.

hex_dump() {
    local s=$1
    local width=${2:-16}
    local i ch byte_str chars hex_line
    local len=${#s}
    local offset=0
    while (( offset < len )); do
        hex_line=""
        chars=""
        for ((i=0; i<width && offset+i<len; i++)); do
            ch=${s[offset+i+1]}
            byte_str=$(printf "%02X" "'$ch")
            hex_line+="$byte_str "
            # Printable?
            local code=$(printf "%d" "'$ch")
            if (( code >= 32 && code < 127 )); then
                chars+=$ch
            else
                chars+="."
            fi
        done
        # Pad hex section.
        while (( ${#hex_line} < width * 3 )); do hex_line+=" "; done
        printf "%08X  %s |%s|\n" $offset "$hex_line" "$chars"
        (( offset += width ))
    done
}

echo "── basic hex dump ──"
hex_dump "Hello, World!"

echo "── multi-line ──"
hex_dump "The quick brown fox jumps over the lazy dog."

echo "── small width ──"
hex_dump "ABC" 4
hex_dump "0123456789" 4

echo "── with control characters ──"
hex_dump $'\x01\x02hello\x7fworld\x0a'

echo "── numeric content ──"
hex_dump "42"
hex_dump "3.14159"

echo "── file content (heredoc) ──"
content=$(cat <<EOF
line1
line2
line3
EOF
)
hex_dump "$content"

echo "── only hex (no chars) ──"
hex_only() {
    local s=$1
    local i ch
    for ((i=1; i<=${#s}; i++)); do
        ch=${s[i]}
        printf "%02X " "'$ch"
    done
    echo
}
hex_only "DEADBEEF"
hex_only "ZSHRS"
