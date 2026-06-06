#!/usr/bin/env zshrs
# 2D matrix multiplication via flat array + row-major indexing.

# A = [[1 2] [3 4]] · B = [[5 6] [7 8]] = [[19 22] [43 50]].

matmul() {
    local n=$1; shift
    # First n*n values are A, next n*n are B.
    local A=()
    local B=()
    local i j k sum
    for ((i = 1; i <= n*n; i++)); do A+=($1); shift; done
    for ((i = 1; i <= n*n; i++)); do B+=($1); shift; done
    local C=()
    for ((i = 0; i < n; i++)); do
        for ((j = 0; j < n; j++)); do
            sum=0
            for ((k = 0; k < n; k++)); do
                (( sum += A[i*n+k+1] * B[k*n+j+1] ))
            done
            C+=($sum)
        done
    done
    # Print as n rows.
    for ((i = 0; i < n; i++)); do
        local row=""
        for ((j = 0; j < n; j++)); do
            row+="${C[i*n+j+1]} "
        done
        echo "$row"
    done
}

echo "── 2x2 · 2x2 ──"
matmul 2  1 2 3 4   5 6 7 8

echo "── identity · M = M ──"
matmul 3  1 0 0 0 1 0 0 0 1   7 8 9 4 5 6 1 2 3

# === ztest assertions ===
# Capture matmul output for 2x2: [[1 2][3 4]] * [[5 6][7 8]] = [[19 22][43 50]]
out=$(matmul 2  1 2 3 4   5 6 7 8)
zassert_contains "$out" "19 22"           "row 1: 19 22"
zassert_contains "$out" "43 50"           "row 2: 43 50"
# Identity * M = M
out=$(matmul 3  1 0 0 0 1 0 0 0 1   7 8 9 4 5 6 1 2 3)
zassert_contains "$out" "7 8 9"           "identity preserves row 1"
zassert_contains "$out" "4 5 6"           "identity preserves row 2"
zassert_contains "$out" "1 2 3"           "identity preserves row 3"
# Zero matrix * anything = zero
out=$(matmul 2  0 0 0 0   1 2 3 4)
zassert_eq "$(echo "$out" | tr -s ' \n' ' ')"  "0 0 0 0 "  "zero * M = zero"
ztest_run
