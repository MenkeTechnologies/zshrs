#!/usr/bin/env zshrs
# Leet speak — encode + decode + variant table.

typeset -A LEET_BASIC
LEET_BASIC=(
    "a" "4"
    "e" "3"
    "i" "1"
    "o" "0"
    "s" "5"
    "t" "7"
)

typeset -A LEET_ADVANCED
LEET_ADVANCED=(
    "a" "@"
    "b" "8"
    "c" "<"
    "d" "[)"
    "e" "3"
    "f" "ph"
    "g" "9"
    "h" "#"
    "i" "!"
    "k" "|<"
    "l" "1"
    "m" '/\/\\'
    "n" '/\/'
    "o" "0"
    "p" "|>"
    "q" "(_,)"
    "r" "|2"
    "s" "$"
    "t" "+"
    "u" "(_)"
    "v" '\/'
    "w" "vv"
    "x" "><"
    "y" '`/'
    "z" "2"
)

# Encode using basic substitution.
leet_basic_encode() {
    local s=$1 out="" i ch ls
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        ls=${ch:l}
        if [[ -n ${LEET_BASIC[$ls]} ]]; then
            out+="${LEET_BASIC[$ls]}"
        else
            out+="$ch"
        fi
    done
    echo "$out"
}

# Encode advanced (multi-char substitutions).
leet_advanced_encode() {
    local s=$1 out="" i ch ls
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        ls=${ch:l}
        if [[ -n ${LEET_ADVANCED[$ls]} ]]; then
            out+="${LEET_ADVANCED[$ls]}"
        else
            out+="$ch"
        fi
    done
    echo "$out"
}

# Decode basic (reverse).
leet_basic_decode() {
    local s=$1 out="" i ch
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        case $ch in
            4) out+="a" ;;
            3) out+="e" ;;
            1) out+="i" ;;
            0) out+="o" ;;
            5) out+="s" ;;
            7) out+="t" ;;
            *) out+="$ch" ;;
        esac
    done
    echo "$out"
}

# Random leetiness — flip each substitutable char with probability.
RANDOM=42
leet_random() {
    local s=$1 out="" i ch ls
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        ls=${ch:l}
        if [[ -n ${LEET_BASIC[$ls]} ]] && (( RANDOM % 2 == 0 )); then
            out+="${LEET_BASIC[$ls]}"
        else
            out+="$ch"
        fi
    done
    echo "$out"
}

echo "── basic encode/decode round-trip ──"
samples=(
    "hello world"
    "the quick brown fox"
    "zshrs is awesome"
    "leet speak"
    "I am a hacker"
    "Operation Aurora"
)
for s in "${samples[@]}"; do
    enc=$(leet_basic_encode "$s")
    dec=$(leet_basic_decode "$enc")
    printf "  in:  %s\n  l33t: %s\n  back: %s   %s\n\n" "$s" "$enc" "$dec" "$([[ ${dec:l} == ${s:l} ]] && echo ✓ || echo ✗)"
done

echo "── advanced encoding ──"
for s in hello zshrs hacker matrix penguin; do
    enc=$(leet_advanced_encode "$s")
    printf "  %-10s → %s\n" "$s" "$enc"
done

echo
echo "── random leet (50% sub) ──"
RANDOM=42
sentences=(
    "i am elite"
    "the matrix has you"
    "follow the white rabbit"
    "wake up neo"
    "knock knock"
)
for s in "${sentences[@]}"; do
    r=$(leet_random "$s")
    printf "  %s\n  %s\n\n" "$s" "$r"
done

echo "── stats ──"
echo "  basic substitutions: ${#LEET_BASIC}"
echo "  advanced subs:       ${#LEET_ADVANCED}"

echo
echo "── lookup table (basic) ──"
for k in "${(@ko)LEET_BASIC}"; do
    printf "  %s → %s\n" "$k" "${LEET_BASIC[$k]}"
done

echo
echo "── leet speak alphabet (advanced) ──"
for k in "${(@ko)LEET_ADVANCED}"; do
    printf "  %s → %s\n" "$k" "${LEET_ADVANCED[$k]}"
done | column -c 80 2>/dev/null || cat

# === ztest assertions ===
zassert_eq "$(leet_basic_encode hello)"        "h3ll0"               "basic hello"
zassert_eq "$(leet_basic_encode 'leet speak')" "l337 5p34k"          "basic leet speak"
zassert_eq "$(leet_basic_decode 'h3ll0')"      "hello"               "basic decode"
zassert_eq "${LEET_BASIC[a]}"                  "4"                   "table a→4"
zassert_eq "${LEET_BASIC[e]}"                  "3"                   "table e→3"
zassert_eq "${LEET_ADVANCED[h]}"               "#"                   "advanced h→#"
zassert_eq "${#LEET_BASIC}"                    6                     "basic table size"
zassert_eq "${#LEET_ADVANCED}"                 25                    "advanced table size"
zassert_eq "$(leet_advanced_encode hello)"     "#3110"               "advanced hello"
ztest_run
