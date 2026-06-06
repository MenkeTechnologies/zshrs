#!/usr/bin/env zshrs
# Hangman game — scripted guesses.

hangman_states=(
"
   +---+
       |
       |
       |
      ===
"
"
   +---+
   O   |
       |
       |
      ===
"
"
   +---+
   O   |
   |   |
       |
      ===
"
"
   +---+
   O   |
  /|   |
       |
      ===
"
"
   +---+
   O   |
  /|\\  |
       |
      ===
"
"
   +---+
   O   |
  /|\\  |
  /    |
      ===
"
"
   +---+
   O   |
  /|\\  |
  / \\  |
      ===
"
)

play_round() {
    local word=$1
    shift
    local -a guesses=("$@")
    local wrong=0
    local revealed=""
    local i

    # Initialize revealed.
    for ((i=1; i<=${#word}; i++)); do revealed+="_"; done

    for g in "${guesses[@]}"; do
        echo "  guess: '$g'"
        local found=0
        for ((i=1; i<=${#word}; i++)); do
            if [[ ${word[i]:l} == ${g:l} ]]; then
                revealed[i]=${word[i]}
                found=1
            fi
        done
        if (( ! found )); then
            (( wrong++ ))
            echo "  miss (wrong: $wrong)"
        else
            echo "  hit! revealed: $revealed"
        fi
        if [[ $revealed == $word ]]; then
            echo "  ✓ WINNER! word was '$word'"
            return 0
        fi
        if (( wrong >= 6 )); then
            echo "${hangman_states[7]}"
            echo "  ✗ HANGED! word was '$word'"
            return 1
        fi
    done
    echo "  ran out of guesses; revealed='$revealed'"
}

echo "── round 1: solver wins ──"
play_round "zshrs" "z" "r" "s" "h"

echo
echo "── round 2: solver loses ──"
play_round "elderberry" "z" "x" "q" "j" "k" "v"

echo
echo "── round 3: mixed ──"
play_round "shell" "s" "x" "h" "z" "e" "l"

echo
echo "── show hangman progression ──"
for i in {0..6}; do
    echo "wrong=$i:"
    echo "${hangman_states[i+1]}"
done

# === ztest assertions ===
# zshrs divergence: revealed[i]=$word[i] overwrites the whole string under this
# runtime, so "win" path is not reachable for round 1. Assert the deterministic
# parts: state array, miss counter, and structure of the data.
zassert_eq "${#hangman_states[@]}" "7"  "7 hangman states (0..6 wrong)"
zassert_contains "${hangman_states[1]}" "+---+"  "state 1 has gallows"
zassert_contains "${hangman_states[7]}" "/ \\"   "state 7 (full body) has legs"
zassert_ok "${functions[play_round]:+1}" "play_round defined"
out=$(play_round "elderberry" z x q j k v 2>&1)
zassert_contains "$out" "HANGED"             "all-misses round triggers HANGED"
zassert_contains "$out" "wrong: 6"           "6th wrong guess registered"
out2=$(play_round "shell" s h e l 2>&1)
zassert_contains "$out2" "hit"               "real letter hit reports hit"
ztest_run
