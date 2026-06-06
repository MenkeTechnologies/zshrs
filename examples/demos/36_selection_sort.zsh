#!/usr/bin/env zshrs
# Selection sort — minimum-find + swap, O(n²).

selection_sort() {
    local arr=("$@")
    local n=${#arr[@]}
    local i j min_idx tmp
    for ((i = 1; i < n; i++)); do
        min_idx=$i
        for ((j = i + 1; j <= n; j++)); do
            (( arr[j] < arr[min_idx] )) && min_idx=$j
        done
        if (( min_idx != i )); then
            tmp=${arr[i]}
            arr[i]=${arr[min_idx]}
            arr[min_idx]=$tmp
        fi
    done
    echo "${arr[@]}"
}

echo "── basic ──"
selection_sort 64 25 12 22 11
echo "── single ──"
selection_sort 42
echo "── two ──"
selection_sort 2 1
echo "── all same ──"
selection_sort 5 5 5

# === ztest assertions ===
zassert_eq "$(selection_sort 64 25 12 22 11)" "11 12 22 25 64" "basic"
zassert_eq "$(selection_sort 42)"             "42"             "singleton"
zassert_eq "$(selection_sort 2 1)"            "1 2"            "two elements"
zassert_eq "$(selection_sort 5 5 5)"          "5 5 5"          "all same"
zassert_eq "$(selection_sort 3 1 4 1 5 9 2 6)" "1 1 2 3 4 5 6 9" "duplicates"
zassert_eq "$(selection_sort 5 4 3 2 1)"      "1 2 3 4 5"      "reversed"
zassert_eq "$(selection_sort 1 2 3 4 5)"      "1 2 3 4 5"      "already sorted"
ztest_run
