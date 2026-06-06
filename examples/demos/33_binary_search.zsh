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

# === ztest assertions ===
zassert_eq "$(bsearch 23 "${sorted[@]}")" "found 23 at index 6"  "mid"
zassert_eq "$(bsearch 5  "${sorted[@]}")" "found 5 at index 2"   "early"
zassert_eq "$(bsearch 2  "${sorted[@]}")" "found 2 at index 1"   "leftmost"
zassert_eq "$(bsearch 91 "${sorted[@]}")" "found 91 at index 10" "rightmost"
zassert_eq "$(bsearch 100 "${sorted[@]}")" "100 not found"       "absent high"
zassert_eq "$(bsearch 13 "${sorted[@]}")"  "13 not found"        "absent gap"
zassert_dies "bsearch 100 ${sorted[@]} >/dev/null" "absent exits nonzero"
ztest_run
