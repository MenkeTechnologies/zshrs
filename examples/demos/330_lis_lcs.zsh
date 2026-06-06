#!/usr/bin/env zshrs
# Longest Increasing Subsequence + Longest Common Subsequence (DP classics).

# LIS via O(n²) DP.
lis() {
    local -a arr
    arr=("$@")
    local n=${#arr}
    typeset -a dp
    dp=()
    local i j
    for ((i=1; i<=n; i++)); do dp[i]=1; done
    for ((i=2; i<=n; i++)); do
        for ((j=1; j<i; j++)); do
            if (( arr[j] < arr[i] && dp[j] + 1 > dp[i] )); then
                dp[i]=$(( dp[j] + 1 ))
            fi
        done
    done
    local max=0
    for ((i=1; i<=n; i++)); do
        (( dp[i] > max )) && max=${dp[i]}
    done
    echo $max
}

# LIS w/ binary-search O(n log n).
lis_nlogn() {
    local -a arr tails
    arr=("$@")
    local n=${#arr}
    tails=()
    local i v lo hi mid
    for ((i=1; i<=n; i++)); do
        v=${arr[i]}
        lo=1
        hi=${#tails}
        while (( lo <= hi )); do
            mid=$(( (lo + hi) / 2 ))
            if (( tails[mid] < v )); then
                lo=$(( mid + 1 ))
            else
                hi=$(( mid - 1 ))
            fi
        done
        tails[lo]=$v
    done
    echo ${#tails}
}

# LCS — DP table.
lcs() {
    local s1=$1 s2=$2
    local m=${#s1} n=${#s2}
    typeset -A dp
    local i j
    for ((i=0; i<=m; i++)); do dp[$i,0]=0; done
    for ((j=0; j<=n; j++)); do dp[0,$j]=0; done
    for ((i=1; i<=m; i++)); do
        for ((j=1; j<=n; j++)); do
            if [[ ${s1[i]} == ${s2[j]} ]]; then
                dp[$i,$j]=$(( dp[$(( i - 1 )),$(( j - 1 ))] + 1 ))
            else
                local a=${dp[$(( i - 1 )),$j]}
                local b=${dp[$i,$(( j - 1 ))]}
                if (( a > b )); then
                    dp[$i,$j]=$a
                else
                    dp[$i,$j]=$b
                fi
            fi
        done
    done
    echo ${dp[$m,$n]}
}

# Reconstruct LCS string.
lcs_str() {
    local s1=$1 s2=$2
    local m=${#s1} n=${#s2}
    typeset -A dp
    local i j
    for ((i=0; i<=m; i++)); do dp[$i,0]=0; done
    for ((j=0; j<=n; j++)); do dp[0,$j]=0; done
    for ((i=1; i<=m; i++)); do
        for ((j=1; j<=n; j++)); do
            if [[ ${s1[i]} == ${s2[j]} ]]; then
                dp[$i,$j]=$(( dp[$(( i - 1 )),$(( j - 1 ))] + 1 ))
            else
                local a=${dp[$(( i - 1 )),$j]}
                local b=${dp[$i,$(( j - 1 ))]}
                if (( a > b )); then
                    dp[$i,$j]=$a
                else
                    dp[$i,$j]=$b
                fi
            fi
        done
    done
    # Trace back.
    local result=""
    i=$m
    j=$n
    while (( i > 0 && j > 0 )); do
        if [[ ${s1[i]} == ${s2[j]} ]]; then
            result="${s1[i]}${result}"
            (( i-- ))
            (( j-- ))
        elif (( dp[$(( i - 1 )),$j] > dp[$i,$(( j - 1 ))] )); then
            (( i-- ))
        else
            (( j-- ))
        fi
    done
    echo "$result"
}

# Edit distance (Levenshtein).
edit_distance() {
    local s1=$1 s2=$2
    local m=${#s1} n=${#s2}
    typeset -A dp
    local i j
    for ((i=0; i<=m; i++)); do dp[$i,0]=$i; done
    for ((j=0; j<=n; j++)); do dp[0,$j]=$j; done
    for ((i=1; i<=m; i++)); do
        for ((j=1; j<=n; j++)); do
            local cost
            if [[ ${s1[i]} == ${s2[j]} ]]; then
                cost=0
            else
                cost=1
            fi
            local del=${dp[$(( i - 1 )),$j]}
            local ins=${dp[$i,$(( j - 1 ))]}
            local sub=${dp[$(( i - 1 )),$(( j - 1 ))]}
            local d1=$(( del + 1 ))
            local d2=$(( ins + 1 ))
            local d3=$(( sub + cost ))
            local min=$d1
            (( d2 < min )) && min=$d2
            (( d3 < min )) && min=$d3
            dp[$i,$j]=$min
        done
    done
    echo ${dp[$m,$n]}
}

echo "── LIS examples ──"
arrays=(
    "10 22 9 33 21 50 41 60"            # → 5
    "1 2 3 4 5"                         # → 5
    "5 4 3 2 1"                         # → 1
    "3 1 4 1 5 9 2 6 5 3 5"             # → ?
    "1 1 1 1 1"                         # → 1
    "10 9 2 5 3 7 101 18"               # → 4
)
for a in "${arrays[@]}"; do
    set -- ${=a}
    l1=$(lis "$@")
    l2=$(lis_nlogn "$@")
    printf "  [%s] → LIS = %d (DP) / %d (n log n) %s\n" "$a" "$l1" "$l2" \
        "$([[ $l1 == $l2 ]] && echo ✓ || echo ✗)"
done

echo
echo "── LCS examples ──"
pairs=(
    "ABCBDAB|BDCAB"
    "AGGTAB|GXTXAYB"
    "ABCDEF|XYZABC"
    "abc|abc"
    "hello|world"
    "ABC|DEF"
)
for p in "${pairs[@]}"; do
    s1="${p%~*}"
    s1="${p%|*}"
    s2="${p#*|}"
    l=$(lcs "$s1" "$s2")
    s=$(lcs_str "$s1" "$s2")
    printf "  '%s' & '%s' → LCS length=%d, string='%s'\n" "$s1" "$s2" "$l" "$s"
done

echo
echo "── edit distance ──"
ed_pairs=(
    "kitten|sitting"          # → 3
    "abc|abc"                 # → 0
    "abc|def"                 # → 3
    "horse|ros"               # → 3
)
for p in "${ed_pairs[@]}"; do
    s1="${p%|*}"
    s2="${p#*|}"
    d=$(edit_distance "$s1" "$s2")
    printf "  edit_distance('%s', '%s') = %d\n" "$s1" "$s2" "$d"
done

echo
echo "── diff context (LCS-based unified diff) ──"
# Simplistic: show insertions/deletions inferred from LCS.
old="A B C D E F G"
new="A B X D E Y G"
lcs_seq=$(lcs_str "${old// /}" "${new// /}")
echo "  old:  $old"
echo "  new:  $new"
echo "  LCS:  $lcs_seq"
echo "  (chars common to both: $lcs_seq)"

echo
echo "── stats ──"
echo "  LIS O(n²)     — DP table"
echo "  LIS O(n log n) — patience sort"
echo "  LCS O(m·n)    — DP table"
echo "  edit distance  — Wagner-Fischer DP"

# === ztest assertions ===
zassert_eq "$(lis 10 22 9 33 21 50 41 60)"   5 "LIS classic"
zassert_eq "$(lis 1 2 3 4 5)"                5 "LIS sorted"
zassert_eq "$(lis 5 4 3 2 1)"                1 "LIS reverse"
zassert_eq "$(lis_nlogn 10 22 9 33 21 50 41 60)" 5 "LIS nlogn"
zassert_eq "$(lis_nlogn 10 9 2 5 3 7 101 18)"    4 "LIS nlogn 2"
zassert_eq "$(lcs ABCBDAB BDCAB)"            4 "LCS classic"
zassert_eq "$(lcs_str ABCBDAB BDCAB)"   "BDAB" "LCS string"
zassert_eq "$(lcs abc abc)"                  3 "LCS identical"
zassert_eq "$(lcs ABC DEF)"                  0 "LCS no common"
zassert_eq "$(edit_distance kitten sitting)" 3 "edit dist Levenshtein"
zassert_eq "$(edit_distance abc abc)"        0 "edit dist identical"
zassert_eq "$(edit_distance horse ros)"      3 "edit dist horse→ros"
ztest_run
