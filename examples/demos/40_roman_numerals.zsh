#!/usr/bin/env zshrs
# Roman numeral conversion — integer ↔ roman.

to_roman() {
    local n=$1 out=""
    local vals=(1000 900 500 400 100 90 50 40 10 9 5 4 1)
    local syms=(M CM D CD C XC L XL X IX V IV I)
    local i
    for ((i = 1; i <= ${#vals[@]}; i++)); do
        while (( n >= vals[i] )); do
            out+="${syms[i]}"
            (( n -= vals[i] ))
        done
    done
    echo "$out"
}

from_roman() {
    local s=$1 total=0 i prev=0 curr
    for ((i = ${#s}; i >= 1; i--)); do
        case ${s[i]} in
            I) curr=1 ;;
            V) curr=5 ;;
            X) curr=10 ;;
            L) curr=50 ;;
            C) curr=100 ;;
            D) curr=500 ;;
            M) curr=1000 ;;
        esac
        if (( curr < prev )); then
            (( total -= curr ))
        else
            (( total += curr ))
        fi
        prev=$curr
    done
    echo "$total"
}

echo "── int → roman ──"
for n in 1 4 9 14 40 49 90 99 400 444 900 999 1000 1994 2024 3999; do
    printf "%4d → %s\n" $n "$(to_roman $n)"
done

echo "── roman → int (round-trip) ──"
for r in I IV IX XIV XL XLIX XC XCIX CD CDXLIV CM CMXCIX M MCMXCIV MMXXIV MMMCMXCIX; do
    n=$(from_roman "$r")
    rt=$(to_roman $n)
    printf "%10s → %4d → %s %s\n" "$r" $n "$rt" "$([[ "$rt" == "$r" ]] && echo OK || echo FAIL)"
done

# === ztest assertions ===
zassert_eq "$(to_roman 1)"     "I"           "1 = I"
zassert_eq "$(to_roman 4)"     "IV"          "4 = IV (subtractive)"
zassert_eq "$(to_roman 9)"     "IX"          "9 = IX"
zassert_eq "$(to_roman 49)"    "XLIX"        "49 = XLIX"
zassert_eq "$(to_roman 1994)"  "MCMXCIV"     "1994 = MCMXCIV"
zassert_eq "$(to_roman 2024)"  "MMXXIV"      "2024 = MMXXIV"
zassert_eq "$(to_roman 3999)"  "MMMCMXCIX"   "3999 = MMMCMXCIX (max)"
zassert_eq "$(from_roman I)"        1        "I = 1"
zassert_eq "$(from_roman IV)"       4        "IV = 4"
zassert_eq "$(from_roman MCMXCIV)"  1994     "MCMXCIV = 1994"
zassert_eq "$(from_roman MMMCMXCIX)" 3999    "MMMCMXCIX = 3999"
# Round trip
zassert_eq "$(to_roman $(from_roman MMXXIV))" "MMXXIV" "round trip MMXXIV"
ztest_run
