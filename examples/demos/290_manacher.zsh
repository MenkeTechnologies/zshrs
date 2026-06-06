#!/usr/bin/env zshrs
# Manacher's algorithm — longest palindromic substring O(n).

manacher() {
    local s=$1
    local n=${#s}
    if (( n == 0 )); then echo "0|"; return; fi

    # Transform: insert # between chars + boundaries. ^#a#b#a#$
    local t="^"
    local i
    for ((i=1; i<=n; i++)); do
        t+="#${s[i]}"
    done
    t+="#\$"
    local tn=${#t}

    # P[i] = palindrome radius centered at t[i].
    typeset -a P
    P=()
    for ((i=1; i<=tn; i++)); do P[i]=0; done

    local center=1 right=1
    for ((i=2; i<tn; i++)); do
        local mirror=$(( 2 * center - i ))
        if (( i < right )); then
            local diff=$(( right - i ))
            local m_p=${P[mirror]}
            if (( m_p < diff )); then
                P[i]=$m_p
            else
                P[i]=$diff
            fi
        fi
        # Expand.
        local pi=${P[i]}
        local left_idx=$(( i - 1 - pi ))
        local right_idx=$(( i + 1 + pi ))
        while (( left_idx >= 1 && right_idx <= tn )) && [[ ${t[left_idx]} == ${t[right_idx]} ]]; do
            (( P[i]++ ))
            left_idx=$(( i - 1 - P[i] ))
            right_idx=$(( i + 1 + P[i] ))
        done
        # Update center/right.
        if (( i + P[i] > right )); then
            center=$i
            right=$(( i + P[i] ))
        fi
    done

    # Find max P[i] → longest palindrome.
    local max_len=0 max_center=0
    for ((i=1; i<=tn; i++)); do
        if (( P[i] > max_len )); then
            max_len=${P[i]}
            max_center=$i
        fi
    done

    # Extract in original string.
    local start=$(( (max_center - max_len) / 2 ))
    local len=$max_len
    local sa=$(( start + 1 ))
    local sb=$(( start + len ))
    if (( sb < sa )); then
        echo "${max_len}|"
    else
        echo "${max_len}|${s[$sa,$sb]}"
    fi
}

echo "── longest palindromic substring ──"
tests=(
    "babad"
    "cbbd"
    "racecar"
    "abacdfgdcaba"
    "forgeeksskeegfor"
    "abcdef"
    "aaaaa"
    "a"
    ""
    "noon at noon"
    "madam in eden Im adam"
    "abacdfgdcabba"
)
for s in "${tests[@]}"; do
    res=$(manacher "$s")
    len="${res%|*}"
    pal="${res#*|}"
    if [[ -z $s ]]; then
        printf "  '' → (empty)\n"
    else
        printf "  '%-25s' → len=%d '%s'\n" "$s" $len "$pal"
    fi
done

echo
echo "── all maximal-palindrome lengths via radius array ──"
test="abacaba"
echo "  string: '$test' (length ${#test})"

# Re-run manacher and dump P array.
s=$test
n=${#s}
t="^"
for ((i=1; i<=n; i++)); do t+="#${s[i]}"; done
t+="#\$"
tn=${#t}

typeset -a P
P=()
for ((i=1; i<=tn; i++)); do P[i]=0; done
center=1; right=1
for ((i=2; i<tn; i++)); do
    mirror=$(( 2 * center - i ))
    if (( i < right )); then
        diff=$(( right - i ))
        m_p=${P[mirror]}
        if (( m_p < diff )); then P[i]=$m_p; else P[i]=$diff; fi
    fi
    pi=${P[i]}
    li=$(( i - 1 - pi ))
    ri=$(( i + 1 + pi ))
    while (( li >= 1 && ri <= tn )) && [[ ${t[li]} == ${t[ri]} ]]; do
        (( P[i]++ ))
        li=$(( i - 1 - P[i] ))
        ri=$(( i + 1 + P[i] ))
    done
    if (( i + P[i] > right )); then
        center=$i
        right=$(( i + P[i] ))
    fi
done

echo "  transformed:  $t"
printf "  radii P[i]:   "
for ((i=1; i<=tn; i++)); do printf "%d " ${P[i]}; done
echo

echo
echo "── count of palindromic substrings ──"
# For each P[i], #palindromes centered at i = (P[i] + 1) / 2.
samples=(abc abcba aaaa abab racecar)
for s in "${samples[@]}"; do
    # Re-run.
    n=${#s}
    t="^"
    for ((i=1; i<=n; i++)); do t+="#${s[i]}"; done
    t+="#\$"
    tn=${#t}
    P=()
    for ((i=1; i<=tn; i++)); do P[i]=0; done
    center=1; right=1
    for ((i=2; i<tn; i++)); do
        mirror=$(( 2 * center - i ))
        if (( i < right )); then
            diff=$(( right - i ))
            m_p=${P[mirror]}
            if (( m_p < diff )); then P[i]=$m_p; else P[i]=$diff; fi
        fi
        pi=${P[i]}
        li=$(( i - 1 - pi ))
        ri=$(( i + 1 + pi ))
        while (( li >= 1 && ri <= tn )) && [[ ${t[li]} == ${t[ri]} ]]; do
            (( P[i]++ ))
            li=$(( i - 1 - P[i] ))
            ri=$(( i + 1 + P[i] ))
        done
        if (( i + P[i] > right )); then
            center=$i
            right=$(( i + P[i] ))
        fi
    done
    total=0
    for ((i=1; i<=tn; i++)); do
        (( total += (P[i] + 1) / 2 ))
    done
    printf "  '%s' has %d palindromic substrings\n" "$s" $total
done

# === ztest assertions ===
# Manacher returns "len|palindrome" pair.
# For 'racecar' (length-7 odd palindrome): max radius = 7
r=$(manacher "racecar")
zassert_eq "${r%|*}" 7 "racecar longest pal length = 7"
# Single char
r=$(manacher "a")
zassert_eq "${r%|*}" 1 "single char len 1"
# Empty string
r=$(manacher "")
zassert_eq "${r%|*}" 0 "empty string len 0"
# aaaaa: all same
r=$(manacher "aaaaa")
zassert_eq "${r%|*}" 5 "aaaaa entire string is palindrome"
# No palindromes longer than 1
r=$(manacher "abcdef")
zassert_eq "${r%|*}" 1 "abcdef has no palindromes >1"
# Transformation includes ^ and # separators
zassert_eq "$t" '^#r#a#c#e#c#a#r#$' "transformed string for 'racecar'"
# count_palindromes for 'aaaa' = 10 (per zshrs observed)
zassert_eq "$total" 10 "racecar has 10 palindromic substrings (last iteration)"
# Final state from the count-palindromes loop is 'racecar' (last sample)
zassert_eq "${samples[-1]}" "racecar" "last sample in count loop"
ztest_run
