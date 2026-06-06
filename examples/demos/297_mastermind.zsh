#!/usr/bin/env zshrs
# Mastermind — scripted player + black/white peg feedback.

COLORS=(R G B Y O P)
CODE_LEN=4

# Generate random secret.
gen_secret() {
    local i
    typeset -ga SECRET
    SECRET=()
    for ((i=0; i<CODE_LEN; i++)); do
        SECRET+=( ${COLORS[$(( RANDOM % ${#COLORS} + 1 ))]} )
    done
}

# Score guess vs secret.
score() {
    local -a guess secret
    guess=($1 $2 $3 $4)
    secret=("${SECRET[@]}")
    local black=0 white=0 i j
    typeset -A used_secret used_guess
    # Black: exact match.
    for ((i=1; i<=CODE_LEN; i++)); do
        if [[ ${guess[i]} == ${secret[i]} ]]; then
            (( black++ ))
            used_secret[$i]=1
            used_guess[$i]=1
        fi
    done
    # White: right color wrong position.
    for ((i=1; i<=CODE_LEN; i++)); do
        [[ -n ${used_guess[$i]+x} ]] && continue
        for ((j=1; j<=CODE_LEN; j++)); do
            [[ -n ${used_secret[$j]+x} ]] && continue
            if [[ ${guess[i]} == ${secret[j]} ]]; then
                (( white++ ))
                used_secret[$j]=1
                break
            fi
        done
    done
    echo "$black $white"
}

# Random guess.
random_guess() {
    local -a g
    local i
    for ((i=0; i<CODE_LEN; i++)); do
        g+=( ${COLORS[$(( RANDOM % ${#COLORS} + 1 ))]} )
    done
    echo "${g[@]}"
}

RANDOM=42

echo "── code length: $CODE_LEN, colors: ${COLORS[*]} ──"

echo
echo "── 5 random scripted rounds ──"
total_solved=0
total_rounds=0
for round in 1 2 3 4 5; do
    gen_secret
    echo "  round $round (secret: ${SECRET[*]}):"
    local solved=0
    local turn
    for turn in 1 2 3 4 5 6 7 8; do
        g=$(random_guess)
        set -- ${=g}
        b_w=$(score $1 $2 $3 $4)
        b=${b_w%% *}
        w=${b_w#* }
        printf "    turn %d: %s %s → B=%d W=%d\n" $turn $1 "$2 $3 $4" $b $w
        if (( b == CODE_LEN )); then
            echo "    ✓ SOLVED on turn $turn"
            solved=1
            (( total_solved++ ))
            break
        fi
    done
    (( total_rounds++ ))
    (( ! solved )) && echo "    ✗ not solved in 8 turns"
done
echo
echo "  solved: $total_solved / $total_rounds"

echo
echo "── 'all same' edge cases ──"
SECRET=(R R R R)
echo "  secret: R R R R"
for g in "R R R R" "G G G G" "R G R G" "R B Y G"; do
    set -- ${=g}
    b_w=$(score $1 $2 $3 $4)
    printf "    guess %s → B=%d W=%d\n" "$g" "${b_w%% *}" "${b_w#* }"
done

echo
echo "── 'all different' edge case ──"
SECRET=(R G B Y)
echo "  secret: R G B Y"
for g in "R G B Y" "Y B G R" "R G B G" "B Y R G" "P O P O"; do
    set -- ${=g}
    b_w=$(score $1 $2 $3 $4)
    printf "    guess %s → B=%d W=%d\n" "$g" "${b_w%% *}" "${b_w#* }"
done

echo
echo "── color frequency in 1000 random secrets ──"
RANDOM=123
typeset -A color_count
for ((g=0; g<1000; g++)); do
    gen_secret
    for c in "${SECRET[@]}"; do
        (( color_count[$c]++ ))
    done
done
total=0
for c in "${COLORS[@]}"; do
    (( total += color_count[$c] ))
done
echo "  total slots: $total (1000 × $CODE_LEN)"
for c in "${COLORS[@]}"; do
    n=${color_count[$c]}
    pct=$(( n * 1000 / total ))
    bar=""
    bw=$(( pct / 4 ))
    for ((b=0; b<bw; b++)); do bar+="█"; done
    printf "  %s: %4d (%2d.%d%%) %s\n" $c $n $((pct/10)) $((pct%10)) "$bar"
done
echo "  (uniform ≈ 16.6% per color)"

# === ztest assertions ===
SECRET=(R G B Y)
zassert_eq "$(score R G B Y)" "4 0"   "exact match all 4 black"
zassert_eq "$(score Y B G R)" "0 4"   "all wrong-position 4 white"
zassert_eq "$(score R G B G)" "3 0"   "3 black 0 white"
zassert_eq "$(score P O P O)" "0 0"   "no match"
SECRET=(R R R R)
zassert_eq "$(score R R R R)" "4 0"   "all same exact"
zassert_eq "$(score G G G G)" "0 0"   "all same no overlap"
zassert_eq "$(score R G R G)" "2 0"   "two of same color, two blacks no whites"
zassert_eq "${#COLORS}" "6"            "6 colors defined"
zassert_eq "$CODE_LEN" "4"             "code length 4"
ztest_run
