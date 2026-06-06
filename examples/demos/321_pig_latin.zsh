#!/usr/bin/env zshrs
# Pig Latin — vowel/consonant rules + reverse.

# Standard rules:
#   1. word starts with vowel → append "way"   (or "ay")
#   2. word starts with consonant cluster → move cluster to end, append "ay"
#   3. preserve capitalization of first letter.

is_vowel() {
    local c=${1:l}
    [[ $c == [aeiou] ]]
}

pig_latin_word() {
    local w=$1
    if [[ -z $w ]]; then echo ""; return; fi
    local lower=${w:l}
    local cap=0
    [[ ${w[1]} != ${w[1]:l} ]] && cap=1
    if is_vowel "${lower[1]}"; then
        # Vowel start: append "way".
        if (( cap )); then
            local first=${w[1]:u}
            local rest=${w[2,-1]}
            echo "${first}${rest}way"
        else
            echo "${lower}way"
        fi
        return
    fi
    # Consonant: find vowel position.
    local i
    for ((i=1; i<=${#lower}; i++)); do
        if is_vowel "${lower[i]}"; then break; fi
    done
    if (( i > ${#lower} )); then
        # No vowel — just append "ay".
        echo "${lower}ay"
        return
    fi
    local prefix_end=$(( i - 1 ))
    local prefix="${lower[1,$prefix_end]}"
    local suffix="${lower[$i,-1]}"
    local out="${suffix}${prefix}ay"
    if (( cap )); then
        out="${out[1]:u}${out[2,-1]}"
    fi
    echo "$out"
}

pig_latin() {
    local s=$1 out=""
    local w
    for w in ${=s}; do
        if [[ -n $out ]]; then out+=" "; fi
        out+="$(pig_latin_word "$w")"
    done
    echo "$out"
}

# Reverse: rough heuristic (won't always work due to ambiguity).
pig_latin_reverse_word() {
    local w=$1 lower=${w:l}
    if [[ $lower == *way ]]; then
        # Vowel start case: strip "way".
        local base="${lower[1,-4]}"
        echo "$base"
        return
    fi
    if [[ $lower == *ay ]]; then
        # Consonant case: ...VC...ay → CV...
        # Try to find shortest prefix where moving consonants to end + ay matches.
        local stripped="${lower[1,-3]}"
        # Find last vowel...
        local i
        for ((i=${#stripped}; i>=1; i--)); do
            if is_vowel "${stripped[i]}"; then break; fi
        done
        if (( i < ${#stripped} )); then
            local last_v_end=$i
            local v_start=$(( i + 1 ))
            local v_end=${#stripped}
            local consonants="${stripped[$v_start,$v_end]}"
            local rest="${stripped[1,$last_v_end]}"
            echo "${consonants}${rest}"
        else
            echo "$stripped"
        fi
        return
    fi
    echo "$w"
}

pig_latin_reverse() {
    local s=$1 out=""
    for w in ${=s}; do
        if [[ -n $out ]]; then out+=" "; fi
        out+="$(pig_latin_reverse_word "$w")"
    done
    echo "$out"
}

echo "── basic word translations ──"
words=(
    pig
    latin
    hello
    apple
    egg
    smile
    string
    rhythm
    Awesome
    eat
    Computer
    Zsh
)
for w in "${words[@]}"; do
    p=$(pig_latin_word "$w")
    printf "  %-12s → %s\n" "$w" "$p"
done

echo
echo "── sentences ──"
sentences=(
    "the quick brown fox"
    "hello world this is pig latin"
    "every owl is special"
    "an elephant ate apples"
    "I am a programmer"
)
for s in "${sentences[@]}"; do
    p=$(pig_latin "$s")
    printf "  in:  %s\n  out: %s\n\n" "$s" "$p"
done

echo "── reverse pig latin → English ──"
pl_inputs=(
    "ellohay"
    "igpay"
    "atinlay"
    "appleway"
    "atway"
    "Awesomeway"
)
for p in "${pl_inputs[@]}"; do
    eng=$(pig_latin_reverse_word "$p")
    printf "  %s → %s\n" "$p" "$eng"
done

echo
echo "── stats ──"
echo "  rules:"
echo "    vowel-start: append 'way'"
echo "    consonant-start: move consonant cluster to end + 'ay'"
echo "    'string' → 'ingstray' (str → end)"
echo "    'eat' → 'eatway'"
echo "    'rhythm' → 'rhythmay' (no vowel, fallback)"

# === ztest assertions ===
zassert_eq "$(pig_latin_word pig)"     "igpay"      "pig → igpay"
zassert_eq "$(pig_latin_word latin)"   "atinlay"    "latin"
zassert_eq "$(pig_latin_word hello)"   "ellohay"    "hello"
zassert_eq "$(pig_latin_word string)"  "ingstray"   "consonant cluster"
zassert_eq "$(pig_latin_word eat)"     "eatway"     "vowel-start"
zassert_eq "$(pig_latin_word rhythm)"  "rhythmay"   "no vowel fallback"
zassert_eq "$(pig_latin_word Awesome)" "Awesomeway" "cap preserved"
zassert_eq "$(pig_latin 'the quick brown fox')" "ethay uickqay ownbray oxfay" "sentence"
zassert_ok "$(is_vowel a && echo 1)"   "is_vowel a"
zassert_eq "$(is_vowel z && echo 1)"   ""           "is_vowel z false"
ztest_run
