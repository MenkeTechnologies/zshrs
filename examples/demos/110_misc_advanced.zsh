#!/usr/bin/env zshrs
# Miscellaneous advanced patterns — getopts, until-loop, select-style menu
# (deterministic — no interactive prompt), repeat, fc/r-equivalents.
# Ported from various Src/builtin.c paths.

echo "── getopts (POSIX-style arg parser) ──"
parse_with_getopts() {
    OPTIND=1
    while getopts "vd:o:" opt; do
        case $opt in
            v) verbose=1 ;;
            d) debug_level=$OPTARG ;;
            o) output=$OPTARG ;;
            ?) echo "unknown opt: $OPTARG" ;;
        esac
    done
    shift $((OPTIND - 1))
    echo "verbose=$verbose debug=$debug_level output=$output remaining=$*"
}

parse_with_getopts -v -d 3 -o /tmp/log.txt arg1 arg2

echo "── until loop ──"
n=0
until (( n >= 5 )); do
    echo "until $n"
    (( n++ ))
done

echo "── repeat N cmd ──"
repeat 3 echo "knock"

echo "── while with complex condition ──"
i=0
sum=0
while (( i < 10 && sum < 30 )); do
    (( sum += i ))
    echo "i=$i sum=$sum"
    (( i++ ))
done
echo "exit: i=$i sum=$sum"

echo "── nested loops with labelled break — emulate via flag ──"
done_flag=0
for ((i = 1; i <= 5; i++)); do
    for ((j = 1; j <= 5; j++)); do
        if (( i * j > 12 )); then
            done_flag=1
            break 2  # break out of both
        fi
        printf "(%d,%d) " $i $j
    done
done
echo "[break fired at i=$i j=$j]"

echo "── continue at outer ──"
for ((i = 1; i <= 4; i++)); do
    for ((j = 1; j <= 4; j++)); do
        (( j == 2 )) && continue
        printf "(%d,%d) " $i $j
    done
    echo
done

echo "── nested for with conditional skip ──"
for x in 1 2 3 4 5 6 7 8 9 10; do
    if (( x % 3 == 0 )); then continue; fi
    if (( x > 7 )); then break; fi
    echo "x=$x"
done

echo "── time builtin ──"
time (
    sum=0
    for ((i = 1; i <= 1000; i++)); do
        (( sum += i ))
    done
    echo "sum=$sum"
) 2>&1 | head -3

echo "── select-like menu, scripted (no stdin needed) ──"
choices=(alpha beta gamma)
pick=2  # simulated user selection
echo "[1] ${choices[1]}"
echo "[2] ${choices[2]}"
echo "[3] ${choices[3]}"
echo "user picked: ${choices[pick]}"

# === ztest assertions ===
# getopts: parse_with_getopts -v -d 3 -o /tmp/log.txt arg1 arg2
# Reset state, re-parse, capture.
verbose=
debug_level=
output=
gout=$(parse_with_getopts -v -d 3 -o /tmp/log.txt arg1 arg2)
zassert_eq "$gout" "verbose=1 debug=3 output=/tmp/log.txt remaining=arg1 arg2"  "getopts result"
# until loop trace 0..4 (5 iterations)
n=0; out=""
until (( n >= 3 )); do out+="$n,"; (( n++ )); done
zassert_eq "$out" "0,1,2,"  "until loop"
# while complex cond left i=9 sum=36 in demo
zassert_eq "${choices[2]}"  "beta"   "1-based array index"
zassert_eq "${choices[pick]}"  "beta"  "subscript via var"
# break 2 fires at i=3 j=5 in demo (output above: "[break fired at i=3 j=5]")
# Re-run small version of the labelled break.
brk_i=0; brk_j=0
for ((i = 1; i <= 5; i++)); do
    for ((j = 1; j <= 5; j++)); do
        if (( i * j > 12 )); then
            brk_i=$i; brk_j=$j
            break 2
        fi
    done
done
zassert_eq "$brk_i" "3"  "break 2 outer index"
zassert_eq "$brk_j" "5"  "break 2 inner index"
# repeat builtin
rep_out=$(repeat 3 echo knock)
zassert_eq "$rep_out" "knock
knock
knock"  "repeat 3"
ztest_run
