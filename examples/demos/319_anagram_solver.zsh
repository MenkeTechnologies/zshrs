#!/usr/bin/env zshrs
# Anagram solver — canonical sort form + dict grouping.

# Canonical form: sort chars asc.
canonical() {
    local s=$1
    s=${s:l}
    # Split chars, sort, join.
    local -a chars
    chars=( ${(s::)s} )
    # Filter non-letters.
    local -a letters
    letters=()
    local c
    for c in "${chars[@]}"; do
        [[ $c == [a-z] ]] && letters+=("$c")
    done
    chars=( "${(@on)letters}" )
    local out=""
    for c in "${chars[@]}"; do out+="$c"; done
    echo "$out"
}

# Dictionary (small).
DICT=(
    act cat tac
    star tars rats arts tsar
    eat ate tea
    listen silent enlist
    dusty study
    night thing
    earth heart hater
    angel angle glean
    save vase
    dread adder
    rose sore eros
    snap pans naps
    stop tops pots opts spot post
    arc car
    ten net
    bus sub
    god dog
    pat tap apt
    bat tab
    wand dawn
)

echo "── canonicalize examples ──"
words=(listen silent enlist tinsel
       cat tac act
       dusty study sturdy)
for w in "${words[@]}"; do
    c=$(canonical "$w")
    printf "  %-10s → %s\n" "$w" "$c"
done

echo
echo "── group anagrams from dictionary ──"
typeset -A groups
for w in "${DICT[@]}"; do
    key=$(canonical "$w")
    if [[ -n ${groups[$key]} ]]; then
        groups[$key]="${groups[$key]} $w"
    else
        groups[$key]="$w"
    fi
done

echo "  (canonical → all words sharing it)"
multi=0
single=0
for key in "${(@ko)groups}"; do
    members=( ${=groups[$key]} )
    if (( ${#members} > 1 )); then
        printf "  %-10s : %s\n" "$key" "${members[*]}"
        (( multi++ ))
    else
        (( single++ ))
    fi
done

echo
echo "  stats: $multi multi-word groups, $single singletons"
echo "  dict size: ${#DICT}"

echo
echo "── find anagrams of a query ──"
queries=(silent stop arc save night nope rose dusty)
for q in "${queries[@]}"; do
    key=$(canonical "$q")
    found="${groups[$key]:-}"
    if [[ -n $found ]]; then
        # Remove query itself.
        members=( ${=found} )
        local matches=()
        for m in "${members[@]}"; do
            [[ $m == $q ]] && continue
            matches+=("$m")
        done
        if (( ${#matches} > 0 )); then
            printf "  '%s' anagrams: %s\n" "$q" "${matches[*]}"
        else
            printf "  '%s' has no dict-anagrams\n" "$q"
        fi
    else
        printf "  '%s' not in dict — canonical=%s\n" "$q" "$key"
    fi
done

echo
echo "── multi-word anagram (phrase-level) ──"
# eg. "listen" ~ "silent" but also "the eyes" ~ "they see"
phrases=(
    "the eyes:they see"
    "elvis:lives"
    "school master:the classroom"
    "astronomer:moon starer"
    "conversation:voices rant on"
    "11 plus 2:12 plus 1"
)
for p in "${phrases[@]}"; do
    p1="${p%:*}"
    p2="${p#*:}"
    c1=$(canonical "${p1// /}")
    c2=$(canonical "${p2// /}")
    mark="✗"
    [[ $c1 == $c2 ]] && mark="✓"
    printf "  '%s' ~ '%s' : %s\n" "$p1" "$p2" "$mark"
done

echo
echo "── character frequency comparison ──"
char_freq() {
    local s=$1 i ch
    s=${s:l}
    typeset -A f
    for ((i=1; i<=${#s}; i++)); do
        ch="${s[i]}"
        [[ $ch == [a-z] ]] && (( f[$ch]++ ))
    done
    local out=""
    for ch in "${(@ko)f}"; do
        out+="${ch}:${f[$ch]} "
    done
    echo "${out% }"
}

for w in listen silent enlist tinsel; do
    printf "  %-10s : %s\n" "$w" "$(char_freq "$w")"
done

echo
echo "── stats ──"
echo "  canonical buckets: ${#groups}"
echo "  largest group: $(for k in "${(@k)groups}"; do
    members=( ${=groups[$k]} )
    echo "${#members} $k"
done | sort -rn | head -1)"

# === ztest assertions ===
zassert_eq "$(canonical listen)"  "eilnst"  "canonical listen"
zassert_eq "$(canonical silent)"  "eilnst"  "canonical silent"
zassert_eq "$(canonical enlist)"  "eilnst"  "canonical enlist"
zassert_eq "$(canonical CAT)"     "act"     "canonical case-insensitive"
zassert_eq "$(canonical sturdy)"  "drstuy"  "canonical sturdy"
zassert_eq "${#DICT}"             55        "dict size"
zassert_eq "$(canonical 'the eyes')"  "$(canonical 'they see')"  "phrase anagram"
zassert_eq "$(canonical elvis)"   "$(canonical lives)"           "elvis~lives"
ztest_run
