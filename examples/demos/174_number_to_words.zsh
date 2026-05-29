#!/usr/bin/env zshrs
# Convert integer to English words (0..999).

ones=(zero one two three four five six seven eight nine
      ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen)
tens=("" "" twenty thirty forty fifty sixty seventy eighty ninety)

num_to_words() {
    local n=$1
    if (( n < 0 )); then
        echo "negative $(num_to_words $((-n)))"
        return
    fi
    if (( n < 20 )); then
        echo ${ones[n+1]}
        return
    fi
    if (( n < 100 )); then
        local t=$(( n / 10 ))
        local r=$(( n % 10 ))
        local out=${tens[t+1]}
        if (( r > 0 )); then
            out+=" ${ones[r+1]}"
        fi
        echo "$out"
        return
    fi
    if (( n < 1000 )); then
        local h=$(( n / 100 ))
        local r=$(( n % 100 ))
        local out="${ones[h+1]} hundred"
        if (( r > 0 )); then
            out+=" and $(num_to_words $r)"
        fi
        echo "$out"
        return
    fi
    if (( n < 1000000 )); then
        local th=$(( n / 1000 ))
        local r=$(( n % 1000 ))
        local out="$(num_to_words $th) thousand"
        if (( r > 0 )); then
            out+=" $(num_to_words $r)"
        fi
        echo "$out"
        return
    fi
    echo "(out of range: $n)"
}

echo "── 0..20 ──"
for n in 0 1 5 9 10 13 17 20; do
    printf "%4d → %s\n" $n "$(num_to_words $n)"
done

echo "── 21..99 ──"
for n in 21 25 30 47 88 99; do
    printf "%4d → %s\n" $n "$(num_to_words $n)"
done

echo "── 100..999 ──"
for n in 100 101 200 250 333 500 777 999; do
    printf "%4d → %s\n" $n "$(num_to_words $n)"
done

echo "── thousands ──"
for n in 1000 1234 9999 25000 100000 999999; do
    printf "%7d → %s\n" $n "$(num_to_words $n)"
done

echo "── classic ──"
for n in 42 1066 1492 1776 1984 2026; do
    printf "%4d → %s\n" $n "$(num_to_words $n)"
done

echo "── negative ──"
for n in -1 -42 -100; do
    printf "%4d → %s\n" $n "$(num_to_words $n)"
done
