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
