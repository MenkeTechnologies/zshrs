#!/usr/bin/env zshrs
# GCD via Euclidean algorithm + LCM via gcd identity.

gcd() {
    local a=$1 b=$2 t
    if (( a < 0 )); then (( a = -a )); fi
    if (( b < 0 )); then (( b = -b )); fi
    while (( b != 0 )); do
        t=$b
        (( b = a % b ))
        a=$t
    done
    echo $a
}

lcm() {
    local a=$1 b=$2
    local g=$(gcd $a $b)
    if (( g == 0 )); then echo 0; return; fi
    echo $(( (a / g) * b ))
}

echo "── gcd ──"
for pair in "12 18" "100 75" "7 13" "0 5" "48 36" "1071 462"; do
    set -- $=pair
    echo "gcd($1, $2) = $(gcd $1 $2)"
done

echo "── lcm ──"
for pair in "4 6" "12 18" "7 13" "5 10" "21 14" "100 75"; do
    set -- $=pair
    echo "lcm($1, $2) = $(lcm $1 $2)"
done

echo "── gcd of N numbers (reduce) ──"
nums=(48 36 60 24 12)
g=${nums[1]}
for ((i = 2; i <= ${#nums[@]}; i++)); do
    g=$(gcd $g ${nums[i]})
done
echo "gcd(${nums[@]}) = $g"

echo "── coprime check (gcd == 1) ──"
for pair in "8 15" "6 9" "17 23" "14 21"; do
    set -- $=pair
    g=$(gcd $1 $2)
    if (( g == 1 )); then
        echo "$1 and $2 are coprime"
    else
        echo "$1 and $2 share gcd $g"
    fi
done
