#!/usr/bin/env zshrs
# Z-function — for each position i, longest prefix of s also starting at i.

z_function() {
    local s=$1
    local n=${#s}
    typeset -ga Z
    Z=()
    local i
    for ((i=1; i<=n; i++)); do Z[i]=0; done
    if (( n == 0 )); then return; fi
    Z[1]=$n
    local l=1 r=1
    for ((i=2; i<=n; i++)); do
        if (( i <= r )); then
            local diff=$(( r - i + 1 ))
            local m_idx=$(( i - l + 1 ))
            local mz=${Z[m_idx]}
            if (( mz < diff )); then
                Z[i]=$mz
            else
                Z[i]=$diff
            fi
        fi
        local zi=${Z[i]}
        while (( i + zi <= n )) && [[ ${s[1 + zi]} == ${s[i + zi]} ]]; do
            (( Z[i]++ ))
            zi=${Z[i]}
        done
        if (( i + Z[i] - 1 > r )); then
            l=$i
            r=$(( i + Z[i] - 1 ))
        fi
    done
}

# Substring search using Z: concatenate pat + sep + text, find Z[i] = |pat|.
z_search() {
    local text=$1 pat=$2
    local combined="${pat}#${text}"
    z_function "$combined"
    typeset -ga MATCHES
    MATCHES=()
    local plen=${#pat}
    local i text_start=$(( plen + 2 ))
    for ((i=text_start; i<=${#combined}; i++)); do
        if (( Z[i] >= plen )); then
            local pos=$(( i - plen - 1 ))
            MATCHES+=($pos)
        fi
    done
}

echo "── Z values ──"
for s in aabcaabxaaaz aaaaa abacabad abc azabazabaza; do
    z_function "$s"
    printf "  s = '%s'\n  Z = [" "$s"
    local i
    for ((i=1; i<=${#s}; i++)); do
        if (( i > 1 )); then printf " "; fi
        printf "%d" ${Z[i]}
    done
    echo "]"
done

echo
echo "── Z-based substring search ──"
tests=(
    "ABABDABACDABABCABAB|ABABC"
    "MISSISSIPPI|ISS"
    "AAAAA|AA"
    "ABABABAB|ABAB"
    "the quick brown fox|the"
    "no match here|xyz"
)
for t in "${tests[@]}"; do
    text="${t%|*}"
    pat="${t#*|}"
    z_search "$text" "$pat"
    printf "  '%s' in '%s':\n" "$pat" "$text"
    if (( ${#MATCHES} == 0 )); then
        echo "    no matches"
    else
        echo "    at: ${MATCHES[*]}"
    fi
done

echo
echo "── period detection ──"
# String has period p if Z[1+p] + p >= n.
periods=(abcabc abcabcabc abababab xyxyxy abcdef aaaaa)
for s in "${periods[@]}"; do
    z_function "$s"
    local n=${#s}
    local p i found=0
    for ((p=1; p<n; p++)); do
        local idx=$(( p + 1 ))
        local z=${Z[idx]}
        if (( z + p >= n )) && (( n % p == 0 )); then
            echo "  '$s' has period $p"
            found=1
            break
        fi
    done
    (( ! found )) && echo "  '$s' aperiodic"
done

echo
echo "── distinct substring count via Z ──"
# (rough) For each suffix s[i..], distinct adds count derived from Z of reversed.
# Simplified: use total - sum-of-Z over suffixes.
distinct_substrings() {
    local s=$1
    local n=${#s}
    local total=$(( n * (n + 1) / 2 ))
    # Distinct = sum over suffixes of (suffix_len - Z[suffix])
    # Approximation: just report total - sum(Z).
    z_function "$s"
    local sum_z=0 i
    for ((i=1; i<=n; i++)); do
        (( sum_z += Z[i] ))
    done
    # This isn't exact but gives a rough lower bound.
    echo "  '$s' (n=$n): total=$total, sum_Z=$sum_z"
}

for s in abc aaa abab banana mississippi; do
    distinct_substrings "$s"
done

echo
echo "── Z vs KMP comparison ──"
echo "  Z-function:  O(n) preprocessing, single pass"
echo "  KMP failure: O(m) preprocessing"
echo "  both:        O(n+m) total string matching"
echo "  Z stores prefix-match length at each pos"
echo "  KMP stores 'border' length (longest proper prefix = suffix)"

echo
echo "── all-occurrences in repetitive text ──"
text="ABABABABABABAB"
pat="ABAB"
z_search "$text" "$pat"
echo "  pat='$pat' in text (len ${#text}):"
echo "  matches at: ${MATCHES[*]}"
echo "  count: ${#MATCHES}"

# === ztest assertions ===
z_function "aabcaabxaaaz"
zassert_eq "${Z[1]}" 12 "Z[1] = full"
zassert_eq "${Z[2]}" 1  "Z[2] = 1"
zassert_eq "${Z[5]}" 3  "Z[5] = 3"
z_search "MISSISSIPPI" "ISS"
zassert_eq "${MATCHES[*]}" "2 5" "ISS in MISSISSIPPI"
z_search "ABABABAB" "ABAB"
zassert_eq "${MATCHES[*]}" "1 3 5" "ABAB occurrences"
z_search "no match here" "xyz"
zassert_eq "${#MATCHES}" 0 "no match"
z_search "ABABABABABABAB" "ABAB"
zassert_eq "${#MATCHES}" 6 "ABAB count in 14-char"
ztest_run
