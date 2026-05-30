#!/usr/bin/env zshrs
# Text wrapping — word-wrap, fill, justify (left/right/center/full).

# Greedy word wrap: stuff as many words as fit in width.
greedy_wrap() {
    local text=$1 width=$2
    local -a words
    words=( ${=text} )
    local out=""
    local line=""
    local w
    for w in "${words[@]}"; do
        if [[ -z $line ]]; then
            line="$w"
        else
            local candidate="$line $w"
            if (( ${#candidate} <= width )); then
                line="$candidate"
            else
                out+="$line"$'\n'
                line="$w"
            fi
        fi
    done
    if [[ -n $line ]]; then
        out+="$line"
    fi
    echo "$out"
}

# Center align (pad to width).
center_align() {
    local text=$1 width=$2
    local -a lines
    lines=( "${(@f)text}" )
    local out=""
    local l
    for l in "${lines[@]}"; do
        local pad=$(( (width - ${#l}) / 2 ))
        local sp=""
        local j
        for ((j=0; j<pad; j++)); do sp+=" "; done
        out+="${sp}${l}"$'\n'
    done
    echo "${out%$'\n'}"
}

# Right align.
right_align() {
    local text=$1 width=$2
    local -a lines
    lines=( "${(@f)text}" )
    local out=""
    local l
    for l in "${lines[@]}"; do
        local pad=$(( width - ${#l} ))
        local sp=""
        local j
        for ((j=0; j<pad; j++)); do sp+=" "; done
        out+="${sp}${l}"$'\n'
    done
    echo "${out%$'\n'}"
}

# Full justify (distribute spaces between words).
full_justify() {
    local text=$1 width=$2
    local -a lines
    lines=( "${(@f)text}" )
    local out=""
    local n=${#lines}
    local i l
    for ((i=1; i<=n; i++)); do
        l="${lines[i]}"
        # Don't justify the last line.
        if (( i == n )) || (( ${#l} >= width )); then
            out+="$l"$'\n'
            continue
        fi
        local -a words
        words=( ${=l} )
        if (( ${#words} <= 1 )); then
            out+="$l"$'\n'
            continue
        fi
        local total_chars=0
        for w in "${words[@]}"; do
            (( total_chars += ${#w} ))
        done
        local gaps=$(( ${#words} - 1 ))
        local spaces_total=$(( width - total_chars ))
        local base_space=$(( spaces_total / gaps ))
        local extra=$(( spaces_total % gaps ))
        local justified=""
        local k
        for ((k=1; k<${#words}; k++)); do
            justified+="${words[k]}"
            local sp_count=$base_space
            if (( k <= extra )); then
                (( sp_count++ ))
            fi
            local s=""
            local j
            for ((j=0; j<sp_count; j++)); do s+=" "; done
            justified+="$s"
        done
        justified+="${words[-1]}"
        out+="$justified"$'\n'
    done
    echo "${out%$'\n'}"
}

# Strip extra whitespace.
collapse_ws() {
    local s=$1
    s="${s//$'\n'/ }"
    s="${s//$'\t'/ }"
    # Collapse multiple spaces.
    while [[ $s == *"  "* ]]; do
        s="${s//  / }"
    done
    s="${s## }"
    s="${s%% }"
    echo "$s"
}

text="The quick brown fox jumps over the lazy dog. The five boxing wizards jump quickly. Pack my box with five dozen liquor jugs."

echo "── original text (length ${#text}) ──"
echo "  $text"

echo
for width in 30 40 50 70; do
    echo "── width $width ──"
    wrapped=$(greedy_wrap "$text" $width)
    echo "$wrapped" | sed 's/^/  /'
    echo
done

echo "── alignment demo (width 40) ──"
narrow_text="zshrs first compiled Unix shell drop-in zsh"
wrapped=$(greedy_wrap "$narrow_text" 30)
echo "  text: '$narrow_text'"
echo
echo "  left-aligned (greedy wrap):"
echo "$wrapped" | sed 's/^/    |/' | sed 's/$/|/'

echo
echo "  centered:"
center_align "$wrapped" 30 | sed 's/^/    |/' | sed 's/$/|/'

echo
echo "  right-aligned:"
right_align "$wrapped" 30 | sed 's/^/    |/' | sed 's/$/|/'

echo
echo "  full-justified:"
full_justify "$wrapped" 30 | sed 's/^/    |/' | sed 's/$/|/'

echo
echo "── whitespace normalization ──"
messy="   too    many     spaces    and$'\t'tabs   "
echo "  raw: '$messy'"
echo "  clean: '$(collapse_ws "$messy")'"

echo
echo "── word count statistics ──"
long_text="zshrs is the first compiled Unix shell written in Rust. It uses bytecode compilation, the fusevm JIT, AOP intercepts, and a persistent worker pool. The compatibility floor is zsh's 130k lines of C source code, which serves as the lower bound for parity, not the upper bound for features."

word_count=$(echo "$long_text" | wc -w | tr -d ' ')
char_count=${#long_text}
line_count=$(echo "$long_text" | wc -l | tr -d ' ')

echo "  text:  $long_text"
echo
echo "  words:      $word_count"
echo "  characters: $char_count"
echo "  lines:      $line_count"

avg_word_len=$(( (char_count - word_count + 1) / word_count ))
echo "  avg word length: $avg_word_len"

echo
echo "── wrap quality metrics ──"
for w in 40 60 80; do
    wrapped=$(greedy_wrap "$long_text" $w)
    line_count=$(echo "$wrapped" | wc -l | tr -d ' ')
    max_len=0
    while IFS= read -r line; do
        if (( ${#line} > max_len )); then max_len=${#line}; fi
    done <<< "$wrapped"
    echo "  width=$w: $line_count lines, longest line $max_len"
done
