#!/usr/bin/env zshrs
# Reverse words in a sentence — preserving word content.

reverse_words() {
    local words=( $=1 )
    local n=${#words[@]}
    local out=""
    local i
    for ((i = n; i >= 1; i--)); do
        out+="${words[i]}"
        (( i > 1 )) && out+=" "
    done
    echo "$out"
}

for s in "the quick brown fox" \
         "hello world" \
         "one two three four five" \
         "single" \
         "a b c d e f g"; do
    echo "'$s' → '$(reverse_words $s)'"
done

# Capitalize first letter of each word.
capitalize_words() {
    local words=( $=1 )
    local out=""
    for w in "${words[@]}"; do
        local first=${w[1]:u}
        local rest=${w[2,-1]:l}
        out+="${first}${rest} "
    done
    echo "${out% }"
}

echo "── capitalize ──"
for s in "hello world" "the quick brown fox" "MIXED case HERE"; do
    echo "'$s' → '$(capitalize_words $s)'"
done
