#!/usr/bin/env zshrs
# Coin change — min coins + count ways DP.

# Min coins to make amount. -1 if impossible.
min_coins() {
    local amount=$1
    shift
    local -a coins
    coins=("$@")
    typeset -a dp
    dp=()
    local i a c v INF=999999
    local idx prev_idx
    for ((a=0; a<=amount; a++)); do
        idx=$(( a + 1 ))
        dp[idx]=$INF
    done
    dp[1]=0
    for ((a=1; a<=amount; a++)); do
        idx=$(( a + 1 ))
        for c in "${coins[@]}"; do
            if (( a >= c )); then
                prev_idx=$(( a - c + 1 ))
                v=${dp[prev_idx]}
                if (( v < INF && v + 1 < ${dp[idx]} )); then
                    dp[idx]=$(( v + 1 ))
                fi
            fi
        done
    done
    local final_idx=$(( amount + 1 ))
    local final_val="${dp[$final_idx]}"
    if (( final_val >= INF )); then
        echo -1
    else
        echo $final_val
    fi
}

which_coins() {
    local amount=$1
    shift
    local -a coins
    coins=("$@")
    typeset -a dp parent
    dp=()
    parent=()
    local i a c v INF=999999
    local idx prev_idx
    for ((a=0; a<=amount; a++)); do
        idx=$(( a + 1 ))
        dp[idx]=$INF
        parent[idx]=0
    done
    dp[1]=0
    for ((a=1; a<=amount; a++)); do
        idx=$(( a + 1 ))
        for c in "${coins[@]}"; do
            if (( a >= c )); then
                prev_idx=$(( a - c + 1 ))
                v=${dp[prev_idx]}
                if (( v < INF && v + 1 < ${dp[idx]} )); then
                    dp[idx]=$(( v + 1 ))
                    parent[idx]=$c
                fi
            fi
        done
    done
    local final_idx=$(( amount + 1 ))
    if (( ${dp[final_idx]} == INF )); then
        typeset -ga COINS_USED
        COINS_USED=()
        return
    fi
    typeset -ga COINS_USED
    COINS_USED=()
    local cur=$amount
    while (( cur > 0 )); do
        local pidx=$(( cur + 1 ))
        local p=${parent[pidx]}
        COINS_USED+=( $p )
        cur=$(( cur - p ))
    done
}

count_ways() {
    local amount=$1
    shift
    local -a coins
    coins=("$@")
    typeset -a dp
    dp=()
    local i idx
    for ((i=0; i<=amount; i++)); do
        idx=$(( i + 1 ))
        dp[idx]=0
    done
    dp[1]=1
    local c a
    for c in "${coins[@]}"; do
        for ((a=c; a<=amount; a++)); do
            idx=$(( a + 1 ))
            local prev_idx=$(( a - c + 1 ))
            (( dp[idx] += ${dp[prev_idx]} ))
        done
    done
    local final_idx=$(( amount + 1 ))
    echo ${dp[final_idx]}
}

echo "── US coins (1 5 10 25) ──"
US_COINS=(1 5 10 25)
for amt in 1 6 11 27 41 99 100; do
    n=$(min_coins $amt "${US_COINS[@]}")
    which_coins $amt "${US_COINS[@]}"
    printf "  amount=%3d : %2d coins  (%s)\n" $amt $n "${COINS_USED[*]}"
done

echo
echo "── unusual coins (1 3 4) — greedy fails ──"
COINS=(1 3 4)
echo "  greedy would: 6 = 4+1+1 = 3 coins"
echo "  optimal:      6 = 3+3   = 2 coins"
for amt in 6 8 11; do
    n=$(min_coins $amt "${COINS[@]}")
    which_coins $amt "${COINS[@]}"
    printf "  amount=%2d : %d coins (%s)\n" $amt $n "${COINS_USED[*]}"
done

echo
echo "── impossible cases ──"
COINS=(2 4 6)
for amt in 1 3 5 7 9; do
    n=$(min_coins $amt "${COINS[@]}")
    if (( n == -1 )); then
        printf "  amount=%d with coins {2,4,6}: IMPOSSIBLE\n" $amt
    else
        printf "  amount=%d: %d coins\n" $amt $n
    fi
done

echo
echo "── count of ways ──"
echo "  US coins (1 5 10 25), amount → ways"
for amt in 1 5 10 25 50 100; do
    w=$(count_ways $amt "${US_COINS[@]}")
    printf "  %3d → %3d ways\n" $amt $w
done

echo
echo "── word problem: pay 47 cents ──"
echo "  US coins (in cents): {1, 5, 10, 25}"
amt=47
n=$(min_coins $amt 1 5 10 25)
which_coins $amt 1 5 10 25
echo "  minimum coins: $n"
echo "  coins: ${COINS_USED[*]}"

echo
echo "── stats ──"
echo "  min coins DP:   O(amount × |coins|)"
echo "  count ways DP:  O(amount × |coins|)"
echo "  Greedy fails when coin system isn't 'canonical'"
echo "  US coins ARE canonical; {1,3,4} is NOT"

# === ztest assertions ===
US=(1 5 10 25)
zassert_eq "$(min_coins 1   "${US[@]}")"  1 "min coins 1"
zassert_eq "$(min_coins 6   "${US[@]}")"  2 "min coins 6"
zassert_eq "$(min_coins 41  "${US[@]}")"  4 "min coins 41"
zassert_eq "$(min_coins 100 "${US[@]}")"  4 "min coins 100"
zassert_eq "$(min_coins 6   1 3 4)"       2 "non-canonical 6"
zassert_eq "$(min_coins 11  1 3 4)"       3 "non-canonical 11"
zassert_eq "$(min_coins 1   2 4 6)"      -1 "impossible"
zassert_eq "$(count_ways 5   "${US[@]}")"   2 "ways 5"
zassert_eq "$(count_ways 10  "${US[@]}")"   4 "ways 10"
zassert_eq "$(count_ways 100 "${US[@]}")" 242 "ways 100"
zassert_eq "$(min_coins 47 1 5 10 25)"   5 "47 cents"
ztest_run
