#!/usr/bin/env zshrs
# Quiz game with scoring — scripted answers.

# Parallel arrays.
qids=(Q1 Q2 Q3 Q4 Q5)
qprompts=(
    "What is 2+2?"
    "Capital of France?"
    "Year zshrs released?"
    "What does ZSH stand for?"
    "Default zsh prompt char?"
)
corrects=("4" "Paris" "2026" "Z Shell" "\$")
answers=("4" "Paris" "2026" "Z Shell" "\$")

score=0
total=${#qids[@]}

for ((i=1; i<=total; i++)); do
    qid=${qids[i]}
    qtxt=${qprompts[i]}
    correct=${corrects[i]}
    given=${answers[i]}
    if [[ $given == $correct ]]; then
        printf "%-3s %s → '%s' ✓\n" $qid "$qtxt" "$given"
        (( score++ ))
    else
        printf "%-3s %s → '%s' ✗ (correct: '%s')\n" $qid "$qtxt" "$given" "$correct"
    fi
done

echo
echo "── result ──"
echo "score: $score/$total"
pct=$(( score * 100 / total ))
echo "percentage: $pct%"

if (( pct >= 80 )); then echo "grade: A"
elif (( pct >= 60 )); then echo "grade: B"
elif (( pct >= 40 )); then echo "grade: C"
else echo "grade: F"
fi

echo
echo "── random-strategy round ──"
RANDOM=42
correct_count=0
for ((round=1; round<=5; round++)); do
    correct_idx=$(( RANDOM % 5 + 1 ))
    guessed_idx=$(( RANDOM % 5 + 1 ))
    if (( correct_idx == guessed_idx )); then (( correct_count++ )); fi
done
echo "random correct: $correct_count/5 (expected ~1)"

# === ztest assertions ===
zassert_eq "$score"    5    "score 5/5"
zassert_eq "$total"    5    "total 5"
zassert_eq "$pct"      100  "percentage 100"
zassert_eq "${#qids}"  5    "5 question ids"
zassert_eq "${corrects[1]}" "4"     "Q1 correct answer"
zassert_eq "${corrects[2]}" "Paris" "Q2 correct answer"
# random with seed 42 deterministic
zassert_ge "$correct_count" 0 "random count >= 0"
zassert_le "$correct_count" 5 "random count <= 5"
ztest_run
