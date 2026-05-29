#!/usr/bin/env zshrs
# Anagram finder via canonical-form (sorted chars) grouping.

canonical() {
    local s=$1
    s=${s:l}
    # Split chars, sort, rejoin.
    local chars=( ${(o)${(s::)s}} )
    echo "${(j::)chars}"
}

typeset -A ANAGRAMS

add_word() {
    local w=$1
    local key=$(canonical "$w")
    ANAGRAMS[$key]="${ANAGRAMS[$key]:+${ANAGRAMS[$key]} }$w"
}

words=(
    listen silent enlist tinsel inlets
    earth heart hater
    cat act tac
    evil live vile veil
    night thing
    derail laired railed
    care race acre
    palm lamp
    iceman cinema anemic
    debit-card bad-credit
)

echo "── adding words ──"
for w in "${words[@]}"; do
    add_word "$w"
done
echo "total groups: ${#ANAGRAMS[@]}"

echo "── anagram groups (2+ members) ──"
for key in ${(ko)ANAGRAMS}; do
    local entry=${ANAGRAMS[$key]}
    local -a members=( ${(s/ /)entry} )
    if (( ${#members[@]} >= 2 )); then
        printf "  %s → %s\n" "$key" "${members[*]}"
    fi
done

echo "── canonical form examples ──"
for w in listen silent stone notes tones; do
    echo "  $w → $(canonical $w)"
done

echo "── total distinct words: ${#words[@]} ──"
echo "── distinct canonical forms: ${#ANAGRAMS[@]} ──"

echo "── is-anagram check ──"
is_anagram() {
    [[ "$(canonical $1)" == "$(canonical $2)" ]]
}
for pair in "listen silent" "hello world" "race care" "abc abd"; do
    set -- $=pair
    if is_anagram "$1" "$2"; then
        echo "  '$1' ↔ '$2' anagram"
    else
        echo "  '$1' ↔ '$2' not anagram"
    fi
done
