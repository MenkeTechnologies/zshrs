#!/usr/bin/env zshrs
# Bubble sort — O(n²) with early-exit on clean pass.

bubble_sort() {
    local arr=("$@")
    local n=${#arr[@]}
    local i j tmp swapped
    for ((i = 0; i < n - 1; i++)); do
        swapped=0
        for ((j = 1; j < n - i; j++)); do
            if (( arr[j] > arr[j+1] )); then
                tmp=${arr[j]}
                arr[j]=${arr[j+1]}
                arr[j+1]=$tmp
                swapped=1
            fi
        done
        (( swapped == 0 )) && break
    done
    echo "${arr[@]}"
}

echo "── unsorted → sorted ──"
bubble_sort 5 2 8 1 9 3 7 4 6
bubble_sort 100 50 25 75 10 90 40 60 80 20
bubble_sort 1 1 1 1 1
bubble_sort 5 4 3 2 1

# === ztest assertions ===
zassert_eq "$(bubble_sort 5 2 8 1 9 3 7 4 6)" "1 2 3 4 5 6 7 8 9" "random ints"
zassert_eq "$(bubble_sort 100 50 25 75 10 90 40 60 80 20)" "10 20 25 40 50 60 75 80 90 100" "round numbers"
zassert_eq "$(bubble_sort 1 1 1 1 1)" "1 1 1 1 1" "duplicates"
zassert_eq "$(bubble_sort 5 4 3 2 1)" "1 2 3 4 5" "reverse"
zassert_eq "$(bubble_sort 1)" "1" "single element"
zassert_eq "$(bubble_sort 2 1)" "1 2" "swap pair"
zassert_eq "$(bubble_sort -3 -1 -2 0)" "-3 -2 -1 0" "negatives"
ztest_run
