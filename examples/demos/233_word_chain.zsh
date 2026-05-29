#!/usr/bin/env zshrs
# Word chain (last letter → first letter) — bounded simulation.

words=(
    apple elephant tiger rabbit turtle eagle gorilla alligator
    ostrich hippo otter raccoon newt toad dolphin narwhal lion
    nightingale emu ungulate eel lemur raven nautilus sloth
)

play_chain() {
    local start=$1 max_len=${2:-10}
    typeset -A used
    used[$start]=1
    local current=$start
    local len=1
    echo "  start: $start"
    while (( len < max_len )); do
        local last=${current[-1]}
        local found=""
        for w in $words; do
            if [[ -z ${used[$w]+x} && ${w[1]:l} == ${last:l} ]]; then
                found=$w
                break
            fi
        done
        if [[ -z $found ]]; then
            echo "  cannot continue from '$current'"
            break
        fi
        echo "  '$current' (→$last) → '$found'"
        used[$found]=1
        current=$found
        (( len++ ))
    done
    echo "  chain length: $len"
}

echo "── chain starting with apple (max 8) ──"
play_chain apple 8

echo
echo "── chain starting with tiger (max 5) ──"
play_chain tiger 5

echo
echo "── chain starting with lion (max 6) ──"
play_chain lion 6

echo
echo "── words by starting letter ──"
typeset -A by_letter
for w in $words; do
    by_letter[${w[1]:l}]="${by_letter[${w[1]:l}]:+${by_letter[${w[1]:l}]} }$w"
done
for k in ${(ko)by_letter}; do
    echo "  $k: ${by_letter[$k]}"
done

echo
echo "── total words: ${#words[@]} ──"
