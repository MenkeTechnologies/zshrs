#!/usr/bin/env zshrs
# Number guessing simulator — binary-search strategy with hot/cold hints.

RANDOM=42

guess_game() {
    local secret=$1 min=$2 max=$3 max_tries=$4
    local tries=0 lo=$min hi=$max
    echo "  secret=$secret (range [$min..$max])"
    while (( tries < max_tries )); do
        (( tries++ ))
        local guess=$(( (lo + hi) / 2 ))
        if (( guess == secret )); then
            echo "  ✓ guess $guess: HIT! (tries=$tries)"
            return 0
        elif (( guess < secret )); then
            echo "  guess $guess: too low"
            lo=$(( guess + 1 ))
        else
            echo "  guess $guess: too high"
            hi=$(( guess - 1 ))
        fi
    done
    echo "  ✗ failed (exhausted $max_tries tries)"
    return 1
}

echo "── game 1: 50 in [1..100], 7 tries ──"
guess_game 50 1 100 7

echo
echo "── game 2: 73 in [1..100] ──"
guess_game 73 1 100 7

echo
echo "── game 3: 1 in [1..1000] ──"
guess_game 1 1 1000 10

echo
echo "── 100-game simulation ──"
won=0
for ((g=1; g<=100; g++)); do
    secret=$(( RANDOM % 100 + 1 ))
    # Use binary search silently.
    lo=1; hi=100; tries=0
    while (( tries < 7 )); do
        (( tries++ ))
        guess=$(( (lo + hi) / 2 ))
        if (( guess == secret )); then
            (( won++ ))
            break
        elif (( guess < secret )); then
            lo=$(( guess + 1 ))
        else
            hi=$(( guess - 1 ))
        fi
    done
done
echo "binary search wins ${won}/100 (need ⌈log₂ 100⌉ = 7 tries)"

echo
echo "── random-strategy comparison ──"
won=0
for ((g=1; g<=100; g++)); do
    secret=$(( RANDOM % 100 + 1 ))
    tries=0
    while (( tries < 7 )); do
        (( tries++ ))
        guess=$(( RANDOM % 100 + 1 ))
        if (( guess == secret )); then
            (( won++ ))
            break
        fi
    done
done
echo "random search wins ${won}/100 (expected ≈ 7%)"

echo
echo "── hot/cold hint variant ──"
hot_cold() {
    local secret=$1
    local prev_dist=999
    for guess in 50 30 40 35 33 34; do
        local dist=$(( secret - guess ))
        (( dist < 0 )) && dist=$(( -dist ))
        local hint
        if (( dist == 0 )); then hint="HIT!"
        elif (( dist < prev_dist )); then hint="hotter"
        else hint="colder"
        fi
        echo "  guess=$guess (dist=$dist) → $hint"
        prev_dist=$dist
        (( dist == 0 )) && break
    done
}
hot_cold 33

# === ztest assertions ===
out1=$(guess_game 50 1 100 7)
zassert_contains "$out1" "HIT! (tries=1)" "50 found at midpoint in 1 try"
out2=$(guess_game 73 1 100 7)
zassert_contains "$out2" "HIT!"            "73 eventually found in 7 tries"
out3=$(guess_game 1 1 1000 10)
zassert_contains "$out3" "HIT! (tries=9)"  "1 in [1..1000] needs 9 tries (binary)"
hc=$(hot_cold 33)
zassert_contains "$hc" "guess=33"          "hot_cold reaches secret"
zassert_contains "$hc" "HIT!"              "hot_cold final HIT"
zassert_contains "$hc" "hotter"            "hot_cold prints 'hotter'"
zassert_contains "$hc" "colder"            "hot_cold prints 'colder'"
ztest_run
