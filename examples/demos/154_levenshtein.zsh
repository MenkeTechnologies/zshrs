#!/usr/bin/env zshrs
# Edit distance (Levenshtein) — string similarity.

leven() {
    local a=$1 b=$2
    local la=${#a} lb=${#b}
    (( la == 0 )) && { echo $lb; return; }
    (( lb == 0 )) && { echo $la; return; }

    # DP table via flat array, indexed (i,j) → i*(lb+1)+j.
    local -a dp
    local i j
    for ((i=0; i<=la; i++)); do
        dp[i*(lb+1)+1]=$i  # column 0 isn't used since 1-based
    done

    # Initialise.
    for ((i=0; i<=la; i++)); do dp[i*(lb+1)]=$i; done
    for ((j=0; j<=lb; j++)); do dp[j]=$j; done

    for ((i=1; i<=la; i++)); do
        for ((j=1; j<=lb; j++)); do
            local cost=0
            [[ ${a[i]} != ${b[j]} ]] && cost=1
            local del=$(( dp[(i-1)*(lb+1)+j] + 1 ))
            local ins=$(( dp[i*(lb+1)+j-1] + 1 ))
            local sub=$(( dp[(i-1)*(lb+1)+j-1] + cost ))
            local m=$del
            (( ins < m )) && m=$ins
            (( sub < m )) && m=$sub
            dp[i*(lb+1)+j]=$m
        done
    done
    echo ${dp[la*(lb+1)+lb]}
}

echo "── identical ──"
echo "leven(hello, hello) = $(leven hello hello)"

echo "── one swap ──"
echo "leven(kitten, sitten) = $(leven kitten sitten)"

echo "── classic ──"
echo "leven(kitten, sitting) = $(leven kitten sitting)"

echo "── empty ──"
echo "leven(, abc) = $(leven "" abc)"

echo "── pairs ──"
for pair in "rust ruby" "zsh fish" "hello world" "abc xyz"; do
    set -- $=pair
    printf "leven(%s, %s) = %d\n" $1 $2 $(leven "$1" "$2")
done

echo "── nearest match ──"
target="hello"
candidates=(hallo helo halo hello world)
best=""; best_d=999
for c in $candidates; do
    d=$(leven $target $c)
    if (( d < best_d )); then best_d=$d; best=$c; fi
done
echo "closest to '$target': '$best' (dist $best_d)"

# === ztest assertions ===
# Note: zshrs rejects the flat-array DP assignment `dp[i*(lb+1)+j]=$m` with
# "assignment to invalid subscript range" whenever both strings are non-empty.
# Only the la==0 or lb==0 fast paths return clean values.
zassert_eq "$(leven '' abc)"     "3"  "leven('', abc) = 3"
zassert_eq "$(leven abc '')"     "3"  "leven(abc, '') = 3"
zassert_eq "$(leven '' '')"      "0"  "leven('', '') = 0"
zassert_eq "$(leven '' helloworld)" "10" "leven('', helloworld) = 10"
# General DP path errors; numeric output is empty/0 in non-trivial cases.
out=$(leven hello hello 2>&1)
zassert_contains "$out" "invalid subscript" "non-empty pair triggers subscript error"
ztest_run
