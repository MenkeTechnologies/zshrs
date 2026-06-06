#!/usr/bin/env zshrs
# Rock-Paper-Scissors — tournament w/ multiple strategies.

# strategy: always_rock / always_paper / always_scissors / random / mirror_prev / cycle.

declare -A WIN_COUNT LOSS_COUNT DRAW_COUNT

throw_random() {
    local r=$(( RANDOM % 3 ))
    case $r in
        0) echo R ;;
        1) echo P ;;
        2) echo S ;;
    esac
}

throw_cycle() {
    local n=$1
    local choices=(R P S)
    echo ${choices[$((n % 3 + 1))]}
}

# Determine winner: returns 0=tie, 1=p1 wins, 2=p2 wins.
duel() {
    local p1=$1 p2=$2
    [[ $p1 == $p2 ]] && return 0
    case "$p1 $p2" in
        "R S"|"P R"|"S P") return 1 ;;
        *) return 2 ;;
    esac
}

play_match() {
    local s1=$1 s2=$2 rounds=$3
    WIN_COUNT[$s1]=0
    WIN_COUNT[$s2]=0
    DRAW_COUNT[$s1]=0
    DRAW_COUNT[$s2]=0
    local i p1 p2 prev1=R prev2=R
    for ((i=1; i<=rounds; i++)); do
        case $s1 in
            always_rock) p1=R ;;
            always_paper) p1=P ;;
            always_scissors) p1=S ;;
            random) p1=$(throw_random) ;;
            cycle) p1=$(throw_cycle $i) ;;
            mirror_prev) p1=$prev2 ;;
        esac
        case $s2 in
            always_rock) p2=R ;;
            always_paper) p2=P ;;
            always_scissors) p2=S ;;
            random) p2=$(throw_random) ;;
            cycle) p2=$(throw_cycle $i) ;;
            mirror_prev) p2=$prev1 ;;
        esac
        duel $p1 $p2
        case $? in
            0) (( DRAW_COUNT[$s1]++ )); (( DRAW_COUNT[$s2]++ )) ;;
            1) (( WIN_COUNT[$s1]++ )) ;;
            2) (( WIN_COUNT[$s2]++ )) ;;
        esac
        prev1=$p1
        prev2=$p2
    done
}

strategies=(always_rock always_paper always_scissors random cycle mirror_prev)
ROUNDS=100
RANDOM=42

echo "── round-robin tournament ($ROUNDS rounds per match) ──"
typeset -A total_wins total_draws
for s1 in "${strategies[@]}"; do
    total_wins[$s1]=0
    total_draws[$s1]=0
done

for s1 in "${strategies[@]}"; do
    for s2 in "${strategies[@]}"; do
        [[ $s1 == $s2 ]] && continue
        play_match $s1 $s2 $ROUNDS
        w=${WIN_COUNT[$s1]}
        d=${DRAW_COUNT[$s1]}
        l=$(( ROUNDS - w - d ))
        (( total_wins[$s1] += w ))
        (( total_draws[$s1] += d ))
        printf "  %-18s vs %-18s : W=%3d L=%3d D=%3d\n" "$s1" "$s2" $w $l $d
    done
done

echo
echo "── leaderboard ──"
# Sort by wins desc.
declare -a names winscores
for s in "${strategies[@]}"; do
    names+=( "$s" )
    winscores+=( "${total_wins[$s]}" )
done
# Bubble sort.
n=${#names}
for ((i=1; i<=n; i++)); do
    for ((j=i+1; j<=n; j++)); do
        if (( winscores[i] < winscores[j] )); then
            tmp=${winscores[i]}; winscores[i]=${winscores[j]}; winscores[j]=$tmp
            tmp2=${names[i]}; names[i]=${names[j]}; names[j]=$tmp2
        fi
    done
done

# Total matches = 5 opponents × 100 = 500.
total_games=$(( (${#strategies} - 1) * ROUNDS ))
for ((i=1; i<=n; i++)); do
    printf "  %d. %-20s W=%4d D=%4d L=%4d   pct=%d%%\n" \
        $i "${names[i]}" "${winscores[i]}" "${total_draws[${names[i]}]}" \
        "$(( total_games - winscores[i] - total_draws[${names[i]}] ))" \
        "$(( winscores[i] * 100 / total_games ))"
done

echo
echo "── pure 50/50 fairness check (random vs random) ──"
RANDOM=42
play_match random random 1000
echo "  random vs random over 1000: W=${WIN_COUNT[random]} D=${DRAW_COUNT[random]}"
echo "  (expected ~333 each if uniform)"

# === ztest assertions ===
# duel return semantics are the deterministic core (independent of PRNG).
duel R R; zassert_eq "$?" 0  "tie: R vs R"
duel R S; zassert_eq "$?" 1  "Rock beats Scissors"
duel P R; zassert_eq "$?" 1  "Paper beats Rock"
duel S P; zassert_eq "$?" 1  "Scissors beats Paper"
duel S R; zassert_eq "$?" 2  "p2 wins: Rock>Scissors"
duel R P; zassert_eq "$?" 2  "p2 wins: Paper>Rock"
duel P S; zassert_eq "$?" 2  "p2 wins: Scissors>Paper"
# Cycle strategy
zassert_eq "$(throw_cycle 0)" "R" "cycle index 0 -> R"
zassert_eq "$(throw_cycle 1)" "P" "cycle index 1 -> P"
zassert_eq "$(throw_cycle 2)" "S" "cycle index 2 -> S"
zassert_eq "$(throw_cycle 3)" "R" "cycle wraps at 3 -> R"
# throw_random returns valid choice
r=$(throw_random)
zassert_match '^[RPS]$' "$r" "throw_random yields R/P/S"
# strategy list
zassert_eq "${#strategies}" 6 "6 strategies"
ztest_run
