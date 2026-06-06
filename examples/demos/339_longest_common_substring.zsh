#!/usr/bin/env zshrs
# Longest common substring — DP table + suffix-array variant.

# DP O(m × n).
lcs_substring() {
    local s1=$1 s2=$2
    local m=${#s1} n=${#s2}
    typeset -A dp
    local i j max_len=0 max_end=0
    for ((i=0; i<=m; i++)); do
        local key="$i,0"
        dp[$key]=0
    done
    for ((j=0; j<=n; j++)); do
        local key="0,$j"
        dp[$key]=0
    done
    for ((i=1; i<=m; i++)); do
        for ((j=1; j<=n; j++)); do
            local key="$i,$j"
            if [[ ${s1[i]} == ${s2[j]} ]]; then
                local prev_i=$(( i - 1 ))
                local prev_j=$(( j - 1 ))
                local prev_key="$prev_i,$prev_j"
                local prev=${dp[$prev_key]}
                dp[$key]=$(( prev + 1 ))
                if (( dp[$key] > max_len )); then
                    max_len=${dp[$key]}
                    max_end=$i
                fi
            else
                dp[$key]=0
            fi
        done
    done
    if (( max_len == 0 )); then
        echo "0|"
        return
    fi
    local start=$(( max_end - max_len + 1 ))
    local end=$max_end
    echo "${max_len}|${s1[$start,$end]}"
}

# Common substrings of length k.
all_common_substrings() {
    local s1=$1 s2=$2 k=$3
    local m=${#s1} n=${#s2}
    typeset -A found
    local i j i_end
    for ((i=1; i<=m-k+1; i++)); do
        i_end=$(( i + k - 1 ))
        local sub="${s1[$i,$i_end]}"
        for ((j=1; j<=n-k+1; j++)); do
            local j_end=$(( j + k - 1 ))
            if [[ ${s2[$j,$j_end]} == $sub ]]; then
                found[$sub]=1
                break
            fi
        done
    done
    echo "${(@k)found}"
}

echo "── LCS examples ──"
pairs=(
    "ABABC|BABCA"
    "ABCDXYZ|XYZABCD"
    "GeeksforGeeks|GeeksQuiz"
    "OldSite|NewSite"
    "abcdef|123abc"
    "GeeksforGeeks|GeeksGreatest"
    "no overlap|completely different"
    "abc|abc"
    "abc|xyz"
)
for p in "${pairs[@]}"; do
    s1="${p%|*}"
    s2="${p#*|}"
    res=$(lcs_substring "$s1" "$s2")
    len="${res%|*}"
    str="${res#*|}"
    printf "  '%-22s' vs '%-22s' → len=%d '%s'\n" "$s1" "$s2" $len "$str"
done

echo
echo "── verify with brute force ──"
brute_lcs() {
    local s1=$1 s2=$2
    local m=${#s1} n=${#s2}
    local max_len=0 max_str=""
    local i j len i_end
    for ((i=1; i<=m; i++)); do
        for ((len=1; i+len-1<=m; len++)); do
            i_end=$(( i + len - 1 ))
            local sub="${s1[$i,$i_end]}"
            for ((j=1; j<=n-len+1; j++)); do
                local j_end=$(( j + len - 1 ))
                if [[ ${s2[$j,$j_end]} == $sub ]] && (( len > max_len )); then
                    max_len=$len
                    max_str=$sub
                fi
            done
        done
    done
    echo "${max_len}|${max_str}"
}

short_tests=("ABC|XBC" "HELLO|WORLD" "abcdef|cdefgh")
for p in "${short_tests[@]}"; do
    s1="${p%|*}"
    s2="${p#*|}"
    dp_res=$(lcs_substring "$s1" "$s2")
    bf_res=$(brute_lcs "$s1" "$s2")
    mark="✓"
    [[ ${dp_res%|*} != ${bf_res%|*} ]] && mark="✗"
    printf "  '%s' & '%s': DP→%s, BF→%s %s\n" "$s1" "$s2" "$dp_res" "$bf_res" "$mark"
done

echo
echo "── all common substrings of length k ──"
s1="ABABCDEFGH"
s2="XYABZCDABEW"
echo "  s1='$s1'"
echo "  s2='$s2'"
for k in 2 3 4; do
    common=$(all_common_substrings "$s1" "$s2" $k)
    printf "  k=%d: " $k
    if [[ -z $common ]]; then
        echo "(none)"
    else
        echo "$common"
    fi
done

echo
echo "── three-way LCS (pairwise) ──"
strs=("ABCDEF" "BCDEFG" "CDEFGH")
echo "  strings: ${strs[*]}"
res12=$(lcs_substring "${strs[1]}" "${strs[2]}")
res23=$(lcs_substring "${strs[2]}" "${strs[3]}")
res13=$(lcs_substring "${strs[1]}" "${strs[3]}")
echo "  LCS(1,2): ${res12#*|}"
echo "  LCS(2,3): ${res23#*|}"
echo "  LCS(1,3): ${res13#*|}"

# All-3 common = intersect them.
res_all=$(lcs_substring "${strs[1]}" "${strs[2]}")
common12="${res_all#*|}"
res_3way=$(lcs_substring "$common12" "${strs[3]}")
echo "  LCS(1,2,3) ≈ ${res_3way#*|}"

echo
echo "── applications ──"
echo "  DNA sequence alignment (longest matching region)"
echo "  diff/patch (file similarity)"
echo "  plagiarism detection"
echo "  data dedup (long shared chunks)"
echo "  database joins on substring matches"

# === ztest assertions ===
zassert_eq "$(lcs_substring ABABC BABCA)"     "4|BABC" "ABABC vs BABCA"
zassert_eq "$(lcs_substring ABCDXYZ XYZABCD)" "4|ABCD" "ABCDXYZ vs XYZABCD"
zassert_eq "$(lcs_substring abc abc)"         "3|abc"  "identical"
zassert_eq "$(lcs_substring abc xyz)"         "0|"     "no common"
zassert_eq "$(brute_lcs ABC XBC)"             "2|BC"   "brute ABC vs XBC"
zassert_eq "$(brute_lcs abcdef cdefgh)"       "4|cdef" "brute long"
zassert_eq "$(lcs_substring OldSite NewSite)" "4|Site" "OldSite vs NewSite"
ztest_run
