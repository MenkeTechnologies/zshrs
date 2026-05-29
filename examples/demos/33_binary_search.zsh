#!/usr/bin/env zshrs
# Binary search over a sorted array — iterative.

bsearch() {
    local target=$1
    shift
    local arr=("$@")
    local lo=1 hi=${#arr[@]} mid
    while (( lo <= hi )); do
        mid=$(( (lo + hi) / 2 ))
        if (( arr[mid] == target )); then
            echo "found $target at index $mid"
            return 0
        elif (( arr[mid] < target )); then
            lo=$(( mid + 1 ))
        else
            hi=$(( mid - 1 ))
        fi
    done
    echo "$target not found"
    return 1
}

sorted=(2 5 8 12 16 23 38 56 72 91)
echo "array: ${sorted[@]}"
for t in 23 5 100 91 2 13; do
    bsearch $t "${sorted[@]}"
done
