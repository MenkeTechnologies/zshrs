#!/usr/bin/env zshrs
# Longest palindromic subsequence — DP table.

# LPS = LCS(s, reverse(s)).
lps() {
    local s=$1
    local n=${#s}
    # Build reversed string.
    local rev="" i
    for ((i=n; i>=1; i--)); do
        rev+="${s[i]}"
    done
    # Standard LCS DP.
    typeset -A dp
    for ((i=0; i<=n; i++)); do
        dp[$i,0]=0
    done
    local j
    for ((j=0; j<=n; j++)); do
        dp[0,$j]=0
    done
    for ((i=1; i<=n; i++)); do
        for ((j=1; j<=n; j++)); do
            local key="$i,$j"
            if [[ ${s[i]} == ${rev[j]} ]]; then
                local prev_i=$(( i - 1 ))
                local prev_j=$(( j - 1 ))
                local prev_key="$prev_i,$prev_j"
                dp[$key]=$(( ${dp[$prev_key]} + 1 ))
            else
                local prev_i=$(( i - 1 ))
                local prev_j=$(( j - 1 ))
                local key_a="$prev_i,$j"
                local key_b="$i,$prev_j"
                local va=${dp[$key_a]}
                local vb=${dp[$key_b]}
                if (( va > vb )); then
                    dp[$key]=$va
                else
                    dp[$key]=$vb
                fi
            fi
        done
    done
    local final="$n,$n"
    echo ${dp[$final]}
}

# Direct DP: lps[i][j] = length of LPS in s[i..j].
lps_direct() {
    local s=$1
    local n=${#s}
    typeset -A dp
    local i j L
    # Single char: length 1.
    for ((i=1; i<=n; i++)); do
        dp[$i,$i]=1
    done
    # Length 2..n.
    for ((L=2; L<=n; L++)); do
        for ((i=1; i<=n-L+1; i++)); do
            j=$(( i + L - 1 ))
            local key="$i,$j"
            local prev_i_p1=$(( i + 1 ))
            local prev_j_m1=$(( j - 1 ))
            local inner_key="${prev_i_p1},${prev_j_m1}"
            if [[ ${s[i]} == ${s[j]} ]]; then
                if (( L == 2 )); then
                    dp[$key]=2
                else
                    dp[$key]=$(( ${dp[$inner_key]} + 2 ))
                fi
            else
                local key_a="${prev_i_p1},$j"
                local key_b="$i,${prev_j_m1}"
                local va=${dp[$key_a]}
                local vb=${dp[$key_b]}
                if (( va > vb )); then
                    dp[$key]=$va
                else
                    dp[$key]=$vb
                fi
            fi
        done
    done
    local key="1,$n"
    echo ${dp[$key]}
}

# Reconstruct LPS string.
lps_string() {
    local s=$1
    local n=${#s}
    typeset -A dp
    local i j L
    for ((i=1; i<=n; i++)); do dp[$i,$i]=1; done
    for ((L=2; L<=n; L++)); do
        for ((i=1; i<=n-L+1; i++)); do
            j=$(( i + L - 1 ))
            local key="$i,$j"
            local ip1=$(( i + 1 ))
            local jm1=$(( j - 1 ))
            local inner="${ip1},${jm1}"
            if [[ ${s[i]} == ${s[j]} ]]; then
                if (( L == 2 )); then
                    dp[$key]=2
                else
                    dp[$key]=$(( ${dp[$inner]} + 2 ))
                fi
            else
                local va=${dp[${ip1},$j]}
                local vb=${dp[$i,${jm1}]}
                if (( va > vb )); then
                    dp[$key]=$va
                else
                    dp[$key]=$vb
                fi
            fi
        done
    done
    # Trace back.
    local result=""
    local lo=1 hi=$n
    while (( lo < hi )); do
        if [[ ${s[lo]} == ${s[hi]} ]]; then
            result="${s[lo]}${result}${s[hi]}"
            (( lo++ ))
            (( hi-- ))
        else
            local va=${dp[$((lo+1)),$hi]}
            local vb=${dp[$lo,$((hi-1))]}
            if (( va > vb )); then
                (( lo++ ))
            else
                (( hi-- ))
            fi
        fi
    done
    if (( lo == hi )); then
        local mid=${#result}
        local half=$(( mid / 2 ))
        result="${result[1,$half]}${s[lo]}${result[$((half+1)),-1]}"
    fi
    echo "$result"
}

echo "── LPS length ──"
strs=(
    "bbbab"           # → 4 (bbbb)
    "cbbd"            # → 2 (bb)
    "agbdba"          # → 5 (abdba)
    "racecar"         # → 7
    "abcde"           # → 1
    "aaaa"            # → 4
    "character"       # → 5 (carac or arara)
    "abcfdghi"        # → 1 (or any single char)
    "abacdfgdcaba"    # → 7+
)
for s in "${strs[@]}"; do
    l=$(lps "$s")
    l2=$(lps_direct "$s")
    str=$(lps_string "$s")
    mark="✓"
    [[ $l != $l2 ]] && mark="✗"
    printf "  '%-13s' → lps=%d (direct=%d %s)  string='%s'\n" "$s" "$l" "$l2" "$mark" "$str"
done

echo
echo "── all palindromic subsequences (brute) ──"
brute_count_palin_subseq() {
    local s=$1
    local n=${#s}
    local count=0
    # 2^n subsequences.
    if (( n > 12 )); then echo "(too many)"; return; fi
    local mask
    for ((mask=1; mask<(1<<n); mask++)); do
        local sub=""
        local i
        for ((i=1; i<=n; i++)); do
            if (( mask & (1 << (i-1)) )); then
                sub+="${s[i]}"
            fi
        done
        # Check palindrome.
        local rev="" j
        for ((j=${#sub}; j>=1; j--)); do
            rev+="${sub[j]}"
        done
        [[ $sub == $rev ]] && (( count++ ))
    done
    echo $count
}

short=(abc aba abca aabb)
for s in "${short[@]}"; do
    c=$(brute_count_palin_subseq "$s")
    printf "  '%s' has %s palindromic subsequences (incl. single chars)\n" "$s" "$c"
done

echo
echo "── min insertions to make palindrome ──"
# min insertions = n - LPS length.
for s in abcd race racecar aabb abacde; do
    l=$(lps "$s")
    n=${#s}
    ins=$(( n - l ))
    printf "  '%s' (n=%d): min insertions = %d (LPS=%d)\n" "$s" $n $ins $l
done

echo
echo "── stats ──"
echo "  Algorithms:"
echo "    LPS via reverse + LCS:  O(n²) time, O(n²) space"
echo "    Direct DP on intervals: O(n²) time, O(n²) space"
echo "    Brute force (2ⁿ):       infeasible for n > 20"
echo
echo "  Related problems:"
echo "    min insertions/deletions to make palindrome"
echo "    min cuts to partition into palindromes"
echo "    count distinct palindromic subsequences (harder)"

# === ztest assertions ===
zassert_eq "$(lps bbbab)"   4 "bbbab LPS"
zassert_eq "$(lps cbbd)"    2 "cbbd LPS"
zassert_eq "$(lps racecar)" 7 "racecar full palindrome"
zassert_eq "$(lps abcde)"   1 "abcde no palindrome"
zassert_eq "$(lps aaaa)"    4 "aaaa all-same"
zassert_eq "$(lps_direct bbbab)"   4 "direct bbbab"
zassert_eq "$(lps_direct racecar)" 7 "direct racecar"
zassert_eq "$(brute_count_palin_subseq abc)"  3 "abc palin count"
zassert_eq "$(brute_count_palin_subseq aba)"  5 "aba palin count"
zassert_eq "$(brute_count_palin_subseq abca)" 7 "abca palin count"
ztest_run
