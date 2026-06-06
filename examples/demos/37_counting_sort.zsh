#!/usr/bin/env zshrs
# Counting sort — O(n + k), works for bounded non-negative integers.

counting_sort() {
    local arr=("$@")
    local n=${#arr[@]}
    local i v max=0
    for v in "${arr[@]}"; do
        (( v > max )) && max=$v
    done
    local count=()
    for ((i = 0; i <= max; i++)); do count+=(0); done
    for v in "${arr[@]}"; do
        (( count[v+1]++ ))
    done
    local out=()
    for ((i = 0; i <= max; i++)); do
        local c=${count[i+1]}
        while (( c-- > 0 )); do out+=($i); done
    done
    echo "${out[@]}"
}

echo "── basic ──"
counting_sort 4 2 2 8 3 3 1
echo "── range 0-9 ──"
counting_sort 9 0 3 7 5 1 6 4 2 8
echo "── single ──"
counting_sort 5
echo "── many duplicates ──"
counting_sort 3 1 3 1 3 1 3 1

# === ztest assertions ===
zassert_eq "$(counting_sort 4 2 2 8 3 3 1)"          "1 2 2 3 3 4 8"      "basic"
zassert_eq "$(counting_sort 9 0 3 7 5 1 6 4 2 8)"    "0 1 2 3 4 5 6 7 8 9" "range 0-9"
zassert_eq "$(counting_sort 5)"                       "5"                  "singleton"
zassert_eq "$(counting_sort 3 1 3 1 3 1 3 1)"        "1 1 1 1 3 3 3 3"    "duplicates"
zassert_eq "$(counting_sort 0)"                       "0"                  "single zero"
zassert_eq "$(counting_sort 0 0 0)"                   "0 0 0"              "all zeros"
ztest_run
