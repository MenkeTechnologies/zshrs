#!/usr/bin/env zshrs
# Suffix array — sort all suffixes; LCP array via Kasai.

build_suffix_array() {
    local s=$1
    local n=${#s}
    typeset -ga SA
    SA=()
    # Naive O(n^2 log n): generate suffixes, sort with index.
    typeset -a tagged
    tagged=()
    local i a b
    for ((i=1; i<=n; i++)); do
        tagged+=("${s[$i,$n]}|$i")
    done
    # Sort by suffix (key is before |).
    sorted_str=$(printf "%s\n" "${tagged[@]}" | sort)
    SA=( ${(f)sorted_str} )
    # Extract indices.
    typeset -ga SA_IDX
    SA_IDX=()
    local entry
    for entry in "${SA[@]}"; do
        SA_IDX+=( "${entry##*|}" )
    done
}

# Kasai LCP — given SA, compute LCP[i] = lcp(s[SA[i]], s[SA[i-1]]).
build_lcp() {
    local s=$1
    local n=${#s}
    # rank[i] = position of suffix-i in SA.
    typeset -A rank
    local i
    for ((i=1; i<=n; i++)); do
        rank[${SA_IDX[i]}]=$i
    done
    typeset -ga LCP
    LCP=()
    for ((i=1; i<=n; i++)); do LCP[i]=0; done
    local h=0 k j
    for ((i=1; i<=n; i++)); do
        if (( rank[$i] > 1 )); then
            k=${SA_IDX[$(( rank[$i] - 1 ))]}
            while (( i + h <= n && k + h <= n )) && [[ ${s[i+h]} == ${s[k+h]} ]]; do
                (( h++ ))
            done
            LCP[${rank[$i]}]=$h
            (( h > 0 )) && (( h-- ))
        fi
    done
}

echo "── suffix array of 'banana' ──"
s="banana"
build_suffix_array "$s"
echo "  string: '$s'"
echo "  sorted suffixes (idx | suffix):"
for ((i=1; i<=${#SA}; i++)); do
    idx=${SA_IDX[i]}
    sb=$idx
    se=${#s}
    printf "    %d : %2d | %s\n" $i $idx "${s[$sb,$se]}"
done

build_lcp "$s"
echo "  LCP array: ${LCP[*]}"

echo
echo "── suffix array of 'mississippi' ──"
s="mississippi"
build_suffix_array "$s"
echo "  string: '$s'"
for ((i=1; i<=${#SA}; i++)); do
    idx=${SA_IDX[i]}
    sb=$idx
    se=${#s}
    printf "    %2d : %2d | %s\n" $i $idx "${s[$sb,$se]}"
done

build_lcp "$s"
echo "  LCP: ${LCP[*]}"

echo
echo "── longest repeated substring (max LCP) ──"
echo "  string: '$s'"
max_lcp=0
max_at=0
for ((i=1; i<=${#LCP}; i++)); do
    if (( LCP[i] > max_lcp )); then
        max_lcp=${LCP[i]}
        max_at=$i
    fi
done
if (( max_lcp > 0 )); then
    idx=${SA_IDX[max_at]}
    sb=$idx
    se=$((idx + max_lcp - 1))
    echo "  longest repeated: '${s[$sb,$se]}' (length $max_lcp)"
else
    echo "  no repeats"
fi

echo
echo "── unique substring count via SA + LCP ──"
# Total substrings = n*(n+1)/2. Subtract repeated overlap = sum of LCP.
samples=(banana abc abcabc mississippi aaaa)
for str in "${samples[@]}"; do
    n=${#str}
    build_suffix_array "$str"
    build_lcp "$str"
    total=$(( n * (n + 1) / 2 ))
    repeats=0
    for ((i=1; i<=${#LCP}; i++)); do
        (( repeats += LCP[i] ))
    done
    unique=$(( total - repeats ))
    printf "  '%-12s' (n=%2d) : total=%3d  repeats=%3d  unique=%3d\n" \
        "$str" $n $total $repeats $unique
done

echo
echo "── binary search a pattern in SA ──"
sa_search() {
    local s=$1 pat=$2
    local n=${#s} m=${#pat}
    local lo=1 hi=${#SA}
    local mid sb se cmp_suffix
    while (( lo <= hi )); do
        mid=$(( (lo + hi) / 2 ))
        idx=${SA_IDX[mid]}
        sb=$idx
        # Take first m chars of suffix.
        local se_m=$(( idx + m - 1 ))
        if (( se_m > n )); then se_m=$n; fi
        cmp_suffix="${s[$sb,$se_m]}"
        if [[ $cmp_suffix == $pat* || $cmp_suffix == $pat ]]; then
            echo $idx
            return 0
        fi
        if [[ $cmp_suffix < $pat ]]; then
            lo=$(( mid + 1 ))
        else
            hi=$(( mid - 1 ))
        fi
    done
    echo "-1"
    return 1
}

s="mississippi"
build_suffix_array "$s"
build_lcp "$s"
for pat in "iss" "miss" "ppi" "abc"; do
    pos=$(sa_search "$s" "$pat")
    if [[ $pos == -1 ]]; then
        echo "  '$pat' not found"
    else
        echo "  '$pat' found at position $pos in '$s'"
    fi
done

# === ztest assertions ===
# banana suffixes in sorted order: a(6) ana(4) anana(2) banana(1) na(5) nana(3)
build_suffix_array "banana"
zassert_eq "${SA_IDX[*]}" "6 4 2 1 5 3"      "SA banana order"
build_lcp "banana"
zassert_eq "${LCP[*]}" "0 1 3 0 0 2"         "LCP banana"
build_suffix_array "mississippi"
build_lcp "mississippi"
zassert_eq "${SA_IDX[1]}" "11"               "miss SA[1]=i at 11"
zassert_eq "${SA_IDX[11]}" "3"               "miss SA[11]=ssissippi at 3"
# "iss" occurs at 2 and 5; the binary search lands on 5.
zassert_eq "$(sa_search mississippi iss)"  "5"   "search iss"
zassert_eq "$(sa_search mississippi miss)" "1"   "search miss"
zassert_eq "$(sa_search mississippi ppi)"  "9"   "search ppi"
zassert_eq "$(sa_search mississippi abc)"  "-1"  "search missing"
ztest_run
