#!/usr/bin/env zshrs
# Negative + ranged array indexing — arr[-1], arr[-3,-1], slices.
# Ported from Src/params.c getindex + Src/subst.c paramsubst slice path.

arr=(alpha beta gamma delta epsilon zeta eta theta)

echo "── single element ──"
echo "first (arr[1]):    ${arr[1]}"
echo "last (arr[-1]):    ${arr[-1]}"
echo "penultimate (arr[-2]): ${arr[-2]}"
echo "third (arr[3]):    ${arr[3]}"
echo "third-from-end:    ${arr[-3]}"

echo "── ranges ──"
echo "[1,3]:    ${arr[1,3]}"
echo "[3,-1]:   ${arr[3,-1]}"
echo "[-3,-1]:  ${arr[-3,-1]}"
echo "[2,5]:    ${arr[2,5]}"

echo "── whole-array forms ──"
echo "all (*):  ${arr[*]}"
echo "all (@):  ${arr[@]}"
echo "len:      ${#arr[@]}"

echo "── slice via offset:length ──"
# zsh: ${arr[@]:OFFSET:LENGTH} — 0-based, like bash.
echo "skip-1, take-3: ${arr[@]:1:3}"
echo "skip-3, take-2: ${arr[@]:3:2}"
echo "from-end-3:     ${arr[@]:-3:3}"

echo "── string indexing on a scalar ──"
s="abcdefghij"
echo "s[1]:     ${s[1]}"
echo "s[-1]:    ${s[-1]}"
echo "s[3,5]:   ${s[3,5]}"
echo "s[-3,-1]: ${s[-3,-1]}"
echo "len:      ${#s}"

echo "── reverse via negative+slice ──"
echo "reversed elements:"
for ((i = ${#arr[@]}; i >= 1; i--)); do
    printf "[%d] %s\n" $i "${arr[i]}"
done

echo "── extract chunks of 2 ──"
for ((i = 1; i <= ${#arr[@]}; i += 2)); do
    end=$(( i + 1 ))
    (( end > ${#arr[@]} )) && end=${#arr[@]}
    echo "chunk: ${arr[i,end]}"
done
