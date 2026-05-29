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
