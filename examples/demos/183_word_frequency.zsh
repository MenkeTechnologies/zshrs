#!/usr/bin/env zshrs
# Word-frequency counter + top-N + filtering.

text="the quick brown fox jumps over the lazy dog the dog barks at the fox the brown fox is quick"

typeset -A freq
for w in $=text; do
    w=${w:l}
    (( freq[$w]++ ))
done

echo "── total unique words: ${#freq[@]} ──"

echo "── alphabetical ──"
for k in ${(ko)freq}; do
    printf "  %-8s %d\n" $k ${freq[$k]}
done

echo
echo "── by frequency (desc) ──"
# Build array of "count<TAB>word" entries, sort numerically reverse.
entries=()
for k in ${(k)freq}; do
    entries+=("$(printf '%05d %s' ${freq[$k]} $k)")
done
print -l ${(On)entries} | head -10

echo
echo "── top 5 most common ──"
print -l ${(On)entries} | head -5 | while IFS=' ' read -r count word; do
    echo "  $word: $((10#${count}))"
done

echo
echo "── words appearing only once ──"
for k in ${(ko)freq}; do
    if (( freq[$k] == 1 )); then echo "  $k"; fi
done

echo
echo "── words containing 'o' ──"
for k in ${(ko)freq}; do
    [[ $k == *o* ]] && echo "  $k (${freq[$k]})"
done

echo
echo "── avg word length ──"
total=0; n=0
for k in ${(k)freq}; do
    (( total += ${#k} * freq[$k] ))
    (( n += freq[$k] ))
done
echo "total chars: $total, total words: $n, avg: $(( total / n )) chars/word"

# === ztest assertions ===
zassert_eq "${#freq[@]}" 11             "unique word count"
zassert_eq "${freq[the]}" 5             "'the' freq"
zassert_eq "${freq[fox]}" 3             "'fox' freq"
zassert_eq "${freq[brown]}" 2           "'brown' freq"
zassert_eq "${freq[at]}" 1              "'at' freq"
zassert_eq "$n" 20                      "total tokens"
zassert_eq "$total" 72                  "sum of char counts"
zassert_eq "$(( total / n ))" 3         "avg word length"
ztest_run
