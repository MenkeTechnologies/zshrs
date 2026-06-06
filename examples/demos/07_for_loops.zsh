#!/usr/bin/env zshrs
# For-loop variants — list, range, c-style, repeat.
echo "── for over list ──"
for x in alpha beta gamma; do
    echo "list: $x"
done

echo "── for over brace range ──"
for i in {1..5}; do
    echo "brace: $i"
done

echo "── for over letter range ──"
for c in {a..e}; do
    echo "letter: $c"
done

echo "── for over stepped range ──"
for i in {0..10..2}; do
    echo "step: $i"
done

echo "── c-style for ──"
for ((i = 0; i < 3; i++)); do
    echo "cstyle: i=$i"
done

echo "── c-style nested ──"
for ((i = 1; i <= 3; i++)); do
    for ((j = 1; j <= 3; j++)); do
        printf "(%d,%d) " $i $j
    done
    echo
done

echo "── repeat N ──"
repeat 4 echo "knock"

# === ztest assertions ===
list_sum=""
for x in alpha beta gamma; do list_sum="${list_sum}${x:0:1}"; done
zassert_eq "$list_sum" "abg" "for over list"
brace_sum=0
for i in {1..5}; do (( brace_sum += i )); done
zassert_eq "$brace_sum" 15 "brace range 1..5 sum"
letter_join=""
for c in {a..e}; do letter_join="${letter_join}${c}"; done
zassert_eq "$letter_join" "abcde" "letter range a..e"
step_sum=0
for i in {0..10..2}; do (( step_sum += i )); done
zassert_eq "$step_sum" 30 "stepped range sum"
cstyle_sum=0
for ((i = 0; i < 3; i++)); do (( cstyle_sum += i )); done
zassert_eq "$cstyle_sum" 3 "c-style 0..2 sum"
nested_total=0
for ((i = 1; i <= 3; i++)); do
    for ((j = 1; j <= 3; j++)); do (( nested_total += i * j )); done
done
zassert_eq "$nested_total" 36 "nested c-style product sum"
ztest_run
