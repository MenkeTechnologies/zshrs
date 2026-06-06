#!/usr/bin/env zshrs
# Quicksort — pure shell, recursive 3-way partition.
qsort() {
    local arr=("$@")
    local len=${#arr[@]}
    (( len <= 1 )) && { echo "${arr[@]}"; return; }
    local pivot=${arr[1]}
    local lt=() gt=() eq=()
    local x
    for x in "${arr[@]}"; do
        if (( x < pivot )); then
            lt+=($x)
        elif (( x > pivot )); then
            gt+=($x)
        else
            eq+=($x)
        fi
    done
    local sorted_lt sorted_gt
    sorted_lt=( $(qsort "${lt[@]}") )
    sorted_gt=( $(qsort "${gt[@]}") )
    echo "${sorted_lt[@]} ${eq[@]} ${sorted_gt[@]}"
}

echo "── small input ──"
qsort 3 1 4 1 5 9 2 6 5 3 5

echo "── duplicates ──"
qsort 5 5 5 5 5

echo "── reverse-sorted input ──"
qsort 10 9 8 7 6 5 4 3 2 1

echo "── single element ──"
qsort 42

echo "── ten random-ish ──"
qsort 50 22 73 19 64 8 91 37 12 88

# === ztest assertions ===
# NOTE: qsort emits stray spaces when lt or gt is empty (echo "${a[@]} ${b[@]} ${c[@]}").
# Compare via tr-squeezed token sequence instead.
sq() { echo "$@" | tr -s ' ' | sed 's/^ //; s/ $//'; }
zassert_eq "$(sq $(qsort 3 1 4 1 5 9 2 6 5 3 5))"     "1 1 2 3 3 4 5 5 5 6 9"      "qsort with dups"
zassert_eq "$(sq $(qsort 5 5 5 5 5))"                  "5 5 5 5 5"                  "all equal"
zassert_eq "$(sq $(qsort 10 9 8 7 6 5 4 3 2 1))"       "1 2 3 4 5 6 7 8 9 10"       "reverse sorted"
zassert_eq "$(sq $(qsort 42))"                         "42"                         "single element"
zassert_eq "$(sq $(qsort 50 22 73 19 64 8 91 37 12 88))" "8 12 19 22 37 50 64 73 88 91" "ten random"
zassert_eq "$(sq $(qsort 1 2 3))"                      "1 2 3"                      "already sorted"
ztest_run
