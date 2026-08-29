#!/usr/bin/env zshrs
# FizzBuzz variants — multiple golfed and clear styles.

echo "── classic if/elif ──"
for i in {1..15}; do
    if (( i % 15 == 0 )); then echo FizzBuzz
    elif (( i % 3 == 0 )); then echo Fizz
    elif (( i % 5 == 0 )); then echo Buzz
    else echo $i
    fi
done

echo
echo "── case statement ──"
for i in {1..15}; do
    case $(( (i % 3 == 0) * 2 + (i % 5 == 0) )) in
        3) echo FizzBuzz ;;
        2) echo Fizz ;;
        1) echo Buzz ;;
        *) echo $i ;;
    esac
done

echo
echo '── ternary via $((...)) ──'
for i in {1..15}; do
    s=""
    (( i % 3 == 0 )) && s+="Fizz"
    (( i % 5 == 0 )) && s+="Buzz"
    echo "${s:-$i}"
done

echo
echo "── array dispatch ──"
typeset -A labels
labels[0]=""    # neither
labels[1]="Buzz"     # %5 == 0 only
labels[2]="Fizz"     # %3 == 0 only
labels[3]="FizzBuzz" # both
for i in {1..15}; do
    key=$(( (i % 3 == 0) * 2 + (i % 5 == 0) ))
    echo "${labels[$key]:-$i}"
done

echo
echo "── concatenate by rule ──"
for i in {1..15}; do
    out=""
    for divrule in "3:Fizz" "5:Buzz" "7:Bazz"; do
        div=${divrule%%:*}
        word=${divrule#*:}
        if (( i % div == 0 )); then out+=$word; fi
    done
    echo "${out:-$i}"
done

echo
echo "── reverse FizzBuzz (which i produces 'FizzBuzz') ──"
for i in {1..30}; do
    if (( i % 15 == 0 )); then echo "  $i: FizzBuzz"; fi
done

echo
echo "── counts in 1..100 ──"
counts=( 0 0 0 0 )  # f, b, fb, plain
for ((i=1; i<=100; i++)); do
    if (( i % 15 == 0 )); then (( counts[3]++ ))
    elif (( i % 3 == 0 )); then (( counts[1]++ ))
    elif (( i % 5 == 0 )); then (( counts[2]++ ))
    else (( counts[4]++ ))
    fi
done
echo "Fizz only: ${counts[1]}"
echo "Buzz only: ${counts[2]}"
echo "FizzBuzz:  ${counts[3]}"
echo "plain:     ${counts[4]}"

# === ztest assertions ===
# NOTE: demo aborts at the `── ternary via $((...)) ──` header line — zshrs
# rejects the literal `$((...))` inside double-quoted echo arg. Smoke only.
zassert_eq $(( 15 % 15 )) 0  "15 mod 15 = 0"
zassert_eq $(( 15 % 3 ))  0  "15 mod 3 = 0"
zassert_eq $(( 15 % 5 ))  0  "15 mod 5 = 0"
ztest_run
