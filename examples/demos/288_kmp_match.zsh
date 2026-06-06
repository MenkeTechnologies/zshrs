#!/usr/bin/env zshrs
# KMP string matching — O(n+m) substring search via failure table.

build_failure() {
    local pat=$1
    local m=${#pat}
    typeset -ga FAIL
    FAIL=()
    FAIL[1]=0
    local i=2 k=0
    local ci ck
    while (( i <= m )); do
        ci=${pat[i]}
        ck=${pat[k+1]}
        if [[ $ci == $ck ]]; then
            (( k++ ))
            FAIL[i]=$k
            (( i++ ))
        elif (( k > 0 )); then
            k=${FAIL[k]}
        else
            FAIL[i]=0
            (( i++ ))
        fi
    done
}

kmp_search() {
    local text=$1 pat=$2
    local n=${#text} m=${#pat}
    if (( m == 0 )); then echo "0"; return; fi
    build_failure "$pat"
    typeset -ga MATCHES
    MATCHES=()
    local i=1 j=0
    local ti pj
    while (( i <= n )); do
        ti=${text[i]}
        pj=${pat[j+1]}
        if [[ $ti == $pj ]]; then
            (( i++ ))
            (( j++ ))
            if (( j == m )); then
                MATCHES+=( $(( i - j )) )
                j=${FAIL[j]}
            fi
        elif (( j > 0 )); then
            j=${FAIL[j]}
        else
            (( i++ ))
        fi
    done
}

echo "── failure table examples ──"
for p in ABABAC ABAB AAAA ABCDABD ABABABA; do
    build_failure "$p"
    printf "  pat='%-10s' fail=[ " "$p"
    for ((k=1; k<=${#p}; k++)); do
        printf "%d " ${FAIL[k]}
    done
    echo "]"
done

echo
echo "── search occurrences ──"
tests=(
    "ABABDABACDABABCABAB|ABABCABAB"
    "ABCABCABCABC|ABC"
    "AAAAAAAAAA|AAA"
    "hello world|world"
    "hello world|xyz"
    "hello world|"
)
for t in "${tests[@]}"; do
    text="${t%~*}"
    text="${t%|*}"
    pat="${t#*|}"
    kmp_search "$text" "$pat"
    n=${#MATCHES}
    printf "  text='%s'\n  pat='%s'\n  found at: %s   (count=%d)\n\n" "$text" "$pat" "${MATCHES[*]:-none}" $n
done

echo "── count overlapping occurrences ──"
overlaps=(
    "AAAAA|AA"
    "ABABAB|AB"
    "ABABAB|ABA"
)
for t in "${overlaps[@]}"; do
    text="${t%|*}"
    pat="${t#*|}"
    kmp_search "$text" "$pat"
    echo "  '$pat' in '$text' at ${MATCHES[*]}  ${#MATCHES} occurrences"
done

echo
echo "── period detection via failure table ──"
# For string s, if (n - FAIL[n]) divides n, then n / (n - FAIL[n]) is the period count.
for s in ABABAB ABCABCABC HELLO ABCDEF AAAA; do
    build_failure "$s"
    n=${#s}
    last=${FAIL[n]}
    period=$(( n - last ))
    if (( n % period == 0 )); then
        echo "  '$s' has period $period (× $(( n / period )))"
    else
        echo "  '$s' aperiodic (longest border: $last)"
    fi
done

echo
echo "── KMP vs naive: same answers? ──"
naive_search() {
    local text=$1 pat=$2 n=${#text} m=${#pat} i j
    typeset -ga NAIVE_MATCHES
    NAIVE_MATCHES=()
    for ((i=1; i<=n-m+1; i++)); do
        local ok=1
        for ((j=1; j<=m; j++)); do
            if [[ ${text[i+j-1]} != ${pat[j]} ]]; then
                ok=0
                break
            fi
        done
        if (( ok )); then NAIVE_MATCHES+=($i); fi
    done
}

samples=(
    "MISSISSIPPI|ISSI"
    "ABABABA|ABA"
    "the quick brown fox|the"
    "AAAAA|AAA"
)
for t in "${samples[@]}"; do
    text="${t%|*}"
    pat="${t#*|}"
    kmp_search "$text" "$pat"
    naive_search "$text" "$pat"
    kmp_str="${MATCHES[*]}"
    naive_str="${NAIVE_MATCHES[*]}"
    printf "  '%s' in '%s':\n" "$pat" "$text"
    printf "    KMP:   %s\n" "$kmp_str"
    printf "    naive: %s   %s\n" "$naive_str" "$([[ $kmp_str == $naive_str ]] && echo ✓ || echo ✗)"
done

# === ztest assertions ===
kmp_search "MISSISSIPPI" "ISSI"
zassert_eq "${MATCHES[*]}" "2 5" "ISSI in MISSISSIPPI at 2,5 (overlapping)"
kmp_search "AAAAA" "AAA"
zassert_eq "${MATCHES[*]}" "1 2 3" "AAA in AAAAA at 1,2,3 (overlapping)"
kmp_search "hello world" "world"
zassert_eq "${MATCHES[1]}" 7 "world starts at index 7 in hello world"
kmp_search "hello world" "xyz"
zassert_eq "${#MATCHES}" 0 "no match found"
kmp_search "ABABABA" "ABA"
zassert_eq "${MATCHES[*]}" "1 3 5" "ABA in ABABABA at 1,3,5"
# Failure table for ABABAC: [0 0 1 2 3 0]
build_failure "ABABAC"
zassert_eq "${FAIL[1]}" 0  "FAIL[1] = 0"
zassert_eq "${FAIL[3]}" 1  "FAIL[3] = 1"
zassert_eq "${FAIL[5]}" 3  "FAIL[5] = 3"
zassert_eq "${FAIL[6]}" 0  "FAIL[6] = 0 (mismatch break)"
# KMP finds substring even when pattern overlaps with itself
kmp_search "ABABABABA" "ABA"
zassert_ge "${#MATCHES}" 3 "ABA in ABABABABA finds ≥3 (overlap)"
ztest_run
