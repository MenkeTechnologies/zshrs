#!/usr/bin/env zshrs
# Insertion sort — stable, O(n²) worst, O(n) on nearly-sorted input.

insertion_sort() {
    local arr=("$@")
    local n=${#arr[@]}
    local i j key
    for ((i = 2; i <= n; i++)); do
        key=${arr[i]}
        j=$(( i - 1 ))
        while (( j >= 1 && arr[j] > key )); do
            arr[j+1]=${arr[j]}
            (( j-- ))
        done
        arr[j+1]=$key
    done
    echo "${arr[@]}"
}

echo "── basic ──"
insertion_sort 7 3 5 1 4 2 8 6
echo "── reversed ──"
insertion_sort 10 9 8 7 6 5 4 3 2 1
echo "── already sorted ──"
insertion_sort 1 2 3 4 5
echo "── duplicates ──"
insertion_sort 3 1 4 1 5 9 2 6 5 3

# === ztest assertions ===
zassert_eq "$(insertion_sort 7 3 5 1 4 2 8 6)"           "1 2 3 4 5 6 7 8"        "basic 8 ints"
zassert_eq "$(insertion_sort 10 9 8 7 6 5 4 3 2 1)"      "1 2 3 4 5 6 7 8 9 10"   "reversed"
zassert_eq "$(insertion_sort 1 2 3 4 5)"                 "1 2 3 4 5"              "already sorted"
zassert_eq "$(insertion_sort 3 1 4 1 5 9 2 6 5 3)"       "1 1 2 3 3 4 5 5 6 9"    "with duplicates"
zassert_eq "$(insertion_sort 1)"                         "1"                       "singleton"
zassert_eq "$(insertion_sort 2 1)"                       "1 2"                     "two elements"
ztest_run
