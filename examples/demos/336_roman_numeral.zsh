#!/usr/bin/env zshrs
# Roman numerals — encode int→Roman, decode Roman→int.

typeset -a VALUES SYMBOLS
VALUES=(1000 900 500 400 100 90 50 40 10 9 5 4 1)
SYMBOLS=(M CM D CD C XC L XL X IX V IV I)

int_to_roman() {
    local n=$1 out="" i v s
    if (( n < 1 || n > 3999 )); then
        echo "out-of-range"
        return
    fi
    for ((i=1; i<=${#VALUES}; i++)); do
        v=${VALUES[i]}
        s=${SYMBOLS[i]}
        while (( n >= v )); do
            out+=$s
            (( n -= v ))
        done
    done
    echo "$out"
}

roman_to_int() {
    local r=$1 i ch nch nval cval total=0
    typeset -A val
    val[I]=1; val[V]=5; val[X]=10; val[L]=50
    val[C]=100; val[D]=500; val[M]=1000
    local n=${#r}
    for ((i=1; i<=n; i++)); do
        ch=${r[i]}
        cval=${val[$ch]}
        if (( i < n )); then
            nch=${r[i+1]}
            nval=${val[$nch]}
            if (( cval < nval )); then
                (( total -= cval ))
                continue
            fi
        fi
        (( total += cval ))
    done
    echo $total
}

is_valid_roman() {
    local r=$1
    [[ -z $r ]] && return 1
    # Only valid chars.
    [[ $r == [IVXLCDM]## ]] || return 1
    # Roundtrip check.
    local n=$(roman_to_int "$r")
    local back=$(int_to_roman $n)
    [[ $back == $r ]]
}

echo "── int → Roman ──"
for n in 1 4 9 14 27 40 49 90 99 444 888 999 1234 1888 2024 3999; do
    r=$(int_to_roman $n)
    printf "  %4d → %s\n" $n "$r"
done

echo
echo "── Roman → int ──"
romans=(I IV IX X XL XC C CM M MCMXCIX MMXXIV)
for r in "${romans[@]}"; do
    n=$(roman_to_int "$r")
    printf "  %-12s → %d\n" "$r" $n
done

echo
echo "── round-trip 1..50 ──"
fail=0
for ((n=1; n<=50; n++)); do
    r=$(int_to_roman $n)
    back=$(roman_to_int "$r")
    if (( back != n )); then
        echo "  ✗ $n → $r → $back"
        (( fail++ ))
    fi
done
echo "  failures: $fail / 50"

echo
echo "── round-trip 1..3999 (every 100) ──"
fail=0
for ((n=1; n<=3999; n+=100)); do
    r=$(int_to_roman $n)
    back=$(roman_to_int "$r")
    if (( back != n )); then
        echo "  ✗ $n → $r → $back"
        (( fail++ ))
    fi
done
echo "  failures: $fail / 40"

echo
echo "── validation ──"
tests=(I IV IX MCMXCIV INVALID IIII XYZ "" "VI")
for t in "${tests[@]}"; do
    if is_valid_roman "$t"; then
        printf "  %-12s : ✓ valid (=%d)\n" "$t" "$(roman_to_int $t)"
    else
        printf "  %-12s : ✗ invalid\n" "$t"
    fi
done

echo
echo "── historic Roman dates ──"
dates=(
    "Julius Caesar assassinated|44 BC"
    "Vesuvius eruption|79"
    "Constantine's victory|312"
    "Fall of Western Rome|476"
    "Charlemagne crowned|800"
    "First crusade|1095"
    "Magna Carta|1215"
    "Gutenberg Bible|1455"
    "Columbus reaches Americas|1492"
    "American Independence|1776"
    "First moon landing|1969"
    "WWW invented|1989"
    "zshrs released|2026"
)
echo "  event                              year  Roman"
for d in "${dates[@]}"; do
    event="${d%|*}"
    year="${d#*|}"
    if [[ $year == *BC* ]]; then
        printf "  %-32s   %4s  %s\n" "$event" "$year" "(pre-Roman)"
    else
        r=$(int_to_roman $year)
        printf "  %-32s   %4d  %s\n" "$event" $year "$r"
    fi
done

echo
echo "── numeral analysis ──"
echo "  symbols: ${SYMBOLS[*]}"
echo "  values:  ${VALUES[*]}"
echo
echo "  subtractive rules:"
echo "    IV = 4 (5-1), IX = 9 (10-1)"
echo "    XL = 40, XC = 90"
echo "    CD = 400, CM = 900"
echo "  no IIII / VIIII, no LL / DD / MM (use M instead)"
echo "  max valid: MMMCMXCIX = 3999"

# === ztest assertions ===
zassert_eq "$(int_to_roman 1)"    "I"        "1 → I"
zassert_eq "$(int_to_roman 4)"    "IV"       "4 → IV"
zassert_eq "$(int_to_roman 9)"    "IX"       "9 → IX"
zassert_eq "$(int_to_roman 1492)" "MCDXCII"  "1492 → MCDXCII"
zassert_eq "$(int_to_roman 2024)" "MMXXIV"   "2024 → MMXXIV"
zassert_eq "$(int_to_roman 3999)" "MMMCMXCIX" "3999 → MMMCMXCIX"
zassert_eq "$(roman_to_int I)"      1     "I → 1"
zassert_eq "$(roman_to_int IV)"     4     "IV → 4"
zassert_eq "$(roman_to_int MCMXCIX)" 1999 "MCMXCIX → 1999"
zassert_eq "$(roman_to_int MMXXIV)" 2024  "MMXXIV → 2024"
zassert_eq "$(int_to_roman 0)"    "out-of-range" "0 invalid"
zassert_eq "$(int_to_roman 4000)" "out-of-range" "4000 invalid"
ztest_run
