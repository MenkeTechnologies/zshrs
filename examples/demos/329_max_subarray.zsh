#!/usr/bin/env zshrs
# Maximum subarray sum — Kadane's algorithm + variants.

# Kadane: classic O(n) max-sum contiguous subarray.
kadane() {
    local -a arr
    arr=("$@")
    local max_so_far=${arr[1]}
    local max_ending_here=${arr[1]}
    typeset -ga RES
    RES=()
    local i
    for ((i=2; i<=${#arr}; i++)); do
        local v=${arr[i]}
        if (( max_ending_here + v > v )); then
            max_ending_here=$(( max_ending_here + v ))
        else
            max_ending_here=$v
        fi
        if (( max_ending_here > max_so_far )); then
            max_so_far=$max_ending_here
        fi
    done
    RES=( $max_so_far )
}

# Kadane with start/end indices.
kadane_with_indices() {
    local -a arr
    arr=("$@")
    local max_so_far=${arr[1]}
    local cur_sum=${arr[1]}
    local start=1 end=1 cur_start=1
    local i
    for ((i=2; i<=${#arr}; i++)); do
        local v=${arr[i]}
        if (( cur_sum + v > v )); then
            cur_sum=$(( cur_sum + v ))
        else
            cur_sum=$v
            cur_start=$i
        fi
        if (( cur_sum > max_so_far )); then
            max_so_far=$cur_sum
            start=$cur_start
            end=$i
        fi
    done
    typeset -ga RES_IDX
    RES_IDX=( $max_so_far $start $end )
}

# Brute force O(n²) for verification.
brute_force_max() {
    local -a arr
    arr=("$@")
    local n=${#arr} max_sum=${arr[1]} i j cur
    for ((i=1; i<=n; i++)); do
        cur=0
        for ((j=i; j<=n; j++)); do
            (( cur += arr[j] ))
            if (( cur > max_sum )); then
                max_sum=$cur
            fi
        done
    done
    echo $max_sum
}

# Maximum subarray that crosses index mid (for divide-and-conquer).
max_crossing() {
    local -a arr
    arr=("$@")
    local mid=$1; shift  # mid is first arg; but we got args differently
    # Actually rework: take mid as separate.
    echo "n/a"
}

echo "── Kadane examples ──"
tests=(
    "-2 1 -3 4 -1 2 1 -5 4"           # → 6 (4 -1 2 1)
    "1 2 3 4 5"                       # → 15
    "-1 -2 -3 -4"                     # → -1
    "5 -1 -2 -3 -4 5"                 # → 5
    "1 -1 1 -1 1 -1 1"                # → 1
    "0 0 0"                           # → 0
    "-5 -1 -8 -9 -10"                 # → -1
    "3 -2 5 -1 6 -3 2 7 -5 2 1"       # → 17
)
for t in "${tests[@]}"; do
    set -- ${=t}
    kadane "$@"
    bf=$(brute_force_max "$@")
    printf "  [%s] → kadane=%d brute=%d %s\n" \
        "$t" "${RES[1]}" "$bf" \
        "$([[ ${RES[1]} == $bf ]] && echo ✓ || echo ✗)"
done

echo
echo "── with start/end indices ──"
arr_strs=(
    "-2 1 -3 4 -1 2 1 -5 4"
    "1 2 -1 3 -4 5 -2 6"
    "10 -100 10 10 10"
)
for s in "${arr_strs[@]}"; do
    set -- ${=s}
    kadane_with_indices "$@"
    printf "  [%s]\n" "$s"
    printf "    max sum: %d, indices [%d..%d]\n" \
        "${RES_IDX[1]}" "${RES_IDX[2]}" "${RES_IDX[3]}"
done

echo
echo "── application: max profit from stock (1 transaction) ──"
# Convert price[] to diffs, then Kadane on diffs.
max_profit() {
    local -a prices diffs
    prices=("$@")
    local i
    for ((i=2; i<=${#prices}; i++)); do
        diffs+=( $(( prices[i] - prices[i-1] )) )
    done
    if (( ${#diffs} == 0 )); then echo 0; return; fi
    kadane "${diffs[@]}"
    local p=${RES[1]}
    if (( p < 0 )); then p=0; fi   # no profit case
    echo $p
}

stock_tests=(
    "7 1 5 3 6 4"     # → 5 (buy at 1, sell at 6)
    "7 6 4 3 1"       # → 0 (always falling)
    "1 2 3 4 5"       # → 4
    "2 4 1"           # → 2
)
for s in "${stock_tests[@]}"; do
    set -- ${=s}
    p=$(max_profit "$@")
    printf "  prices=[%s] → max profit = %d\n" "$s" "$p"
done

echo
echo "── multiple subarrays (k=2 disjoint) ──"
# Toy: find 2 disjoint subarrays w/ max total. Greedy: remove max, repeat.
# (Simplistic — not optimal in general but works for examples.)
arr=(-2 1 -3 4 -1 2 1 -5 4)
echo "  array: ${arr[*]}"
kadane_with_indices "${arr[@]}"
echo "  1st max: ${RES_IDX[1]} at [${RES_IDX[2]}..${RES_IDX[3]}]"

# Zero out that range and re-run.
for ((i=${RES_IDX[2]}; i<=${RES_IDX[3]}; i++)); do
    arr[i]=0
done
echo "  zeroed range: ${arr[*]}"
kadane_with_indices "${arr[@]}"
echo "  2nd max: ${RES_IDX[1]} at [${RES_IDX[2]}..${RES_IDX[3]}]"

echo
echo "── circular variant (allow wrap) ──"
# Max circular = max(kadane(arr), total - kadane(-arr))
circular_max() {
    local -a arr neg
    arr=("$@")
    local i
    for ((i=1; i<=${#arr}; i++)); do
        neg+=( $(( -arr[i] )) )
    done
    kadane "${arr[@]}"
    local linear=${RES[1]}
    kadane "${neg[@]}"
    local min_sub_neg=${RES[1]}
    local total=0
    for v in "${arr[@]}"; do
        (( total += v ))
    done
    local circular=$(( total + min_sub_neg ))
    # All negative case: stick with linear.
    if (( circular == 0 && linear < 0 )); then
        echo $linear
    elif (( circular > linear )); then
        echo $circular
    else
        echo $linear
    fi
}

c_tests=(
    "1 -2 3 -2"       # → 3
    "5 -3 5"          # → 10 (wrap)
    "3 -1 2 -1"       # → 4
    "-3 -2 -3"        # → -2
)
for t in "${c_tests[@]}"; do
    set -- ${=t}
    r=$(circular_max "$@")
    printf "  [%s] → circular max = %d\n" "$t" "$r"
done

echo
echo "── stats ──"
echo "  Kadane complexity:  O(n) time, O(1) space"
echo "  Brute-force:        O(n²)"
echo "  Application:        stock profit, signal peaks, ML segmentation"

# === ztest assertions ===
kadane -2 1 -3 4 -1 2 1 -5 4
zassert_eq "${RES[1]}" 6 "kadane mixed"
kadane 1 2 3 4 5
zassert_eq "${RES[1]}" 15 "kadane all pos"
kadane -1 -2 -3 -4
zassert_eq "${RES[1]}" -1 "kadane all neg"
kadane 0 0 0
zassert_eq "${RES[1]}" 0  "kadane zeros"
zassert_eq "$(brute_force_max 3 -2 5 -1 6 -3 2 7 -5 2 1)" 17 "brute force"
kadane_with_indices -2 1 -3 4 -1 2 1 -5 4
zassert_eq "${RES_IDX[1]}" 6 "kadane idx sum"
zassert_eq "${RES_IDX[2]}" 4 "kadane idx start"
zassert_eq "${RES_IDX[3]}" 7 "kadane idx end"
zassert_eq "$(max_profit 7 1 5 3 6 4)" 5 "stock profit"
zassert_eq "$(max_profit 7 6 4 3 1)"   0 "stock no profit"
zassert_eq "$(circular_max 5 -3 5)" 10 "circular wrap"
ztest_run
