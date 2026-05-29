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
