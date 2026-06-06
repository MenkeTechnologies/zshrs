#!/usr/bin/env zshrs
# String ↔ integer conversion — manual char-by-char.

atoi() {
    local s=$1 sign=1 n=0 i ch d
    if [[ ${s[1]} == "-" ]]; then sign=-1; s=${s[2,-1]}; fi
    if [[ ${s[1]} == "+" ]]; then s=${s[2,-1]}; fi
    for ((i = 1; i <= ${#s}; i++)); do
        ch=${s[i]}
        case $ch in
            [0-9]) d=$(( $(printf "%d" "'$ch") - 48 )); (( n = n * 10 + d )) ;;
            *)     echo "invalid: $1"; return 1 ;;
        esac
    done
    echo $(( sign * n ))
}

itoa() {
    local n=$1 sign="" out=""
    if (( n < 0 )); then sign="-"; (( n = -n )); fi
    if (( n == 0 )); then echo 0; return; fi
    while (( n > 0 )); do
        local d=$(( n % 10 ))
        out="${d}${out}"
        (( n /= 10 ))
    done
    echo "${sign}${out}"
}

echo "── atoi ──"
for s in "42" "-7" "+13" "0" "1000000" "99999"; do
    echo "atoi($s) = $(atoi $s)"
done

echo "── itoa ──"
for n in 42 -7 13 0 1000000 99999; do
    echo "itoa($n) = $(itoa $n)"
done

echo "── round-trip ──"
for n in 0 1 -1 42 -42 1000 -9999; do
    s=$(itoa $n)
    back=$(atoi "$s")
    echo "$n → '$s' → $back $([[ $n -eq $back ]] && echo OK || echo FAIL)"
done

# === ztest assertions ===
zassert_eq "$(atoi 42)"       42       "atoi 42"
zassert_eq "$(atoi -7)"       -7       "atoi -7"
zassert_eq "$(atoi +13)"      13       "atoi +13"
zassert_eq "$(atoi 0)"        0        "atoi 0"
zassert_eq "$(atoi 1000000)"  1000000  "atoi 1000000"
zassert_eq "$(itoa 42)"       42       "itoa 42"
zassert_eq "$(itoa -7)"       -7       "itoa -7"
zassert_eq "$(itoa 0)"        0        "itoa 0"
zassert_eq "$(itoa 1000000)"  1000000  "itoa 1e6"
zassert_eq "$(atoi $(itoa 12345))"  12345 "round-trip 12345"
zassert_eq "$(atoi $(itoa -9999))"  -9999 "round-trip -9999"
ztest_run
