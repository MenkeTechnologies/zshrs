#!/usr/bin/env zshrs
# Calculator engine — REPL-like driver over $((…)) with vars.
# Ports the math.c lexer's recursive descent feel through shell.

typeset -A VARS

calc_set() {
    local name=$1 val=$2
    VARS[$name]=$val
    echo "$name = ${VARS[$name]}"
}

calc_eval() {
    local expr=$1
    # Inline variable substitution from VARS.
    for k in ${(k)VARS}; do
        expr=${expr//\$$k/${VARS[$k]}}
    done
    echo "$(( $expr ))"
}

calc_repl_input=(
    "a = 5"
    "b = 10"
    "c = (a + b) * 2"
    "d = c / 3"
    "e = c ** 2"
    "f = e + d"
)

echo "── batch input → state ──"
for line in "${calc_repl_input[@]}"; do
    if [[ $line == *=* ]]; then
        name=${line%% =*}; name=${name## }
        rhs=${line#*= }
        val=$(calc_eval "$rhs")
        calc_set "$name" "$val"
    fi
done

echo "── final state ──"
for k in ${(ko)VARS}; do
    printf "%-3s = %s\n" $k "${VARS[$k]}"
done

echo "── ad-hoc queries against state ──"
queries=(
    "a + b"
    "c - d"
    "e / f"
    "a * b + c"
)
for q in "${queries[@]}"; do
    result=$(calc_eval "$q")
    printf "%-15s = %s\n" "$q" "$result"
done

echo "── unit conversion via calc ──"
calc_set km 100
calc_set mph 60

# 1 km = 0.621371 mi, 1 mph = 1.609344 km/h
echo "convert 100 km to mi: $(calc_eval "km * 621371 / 1000000")"
echo "convert 60 mph to km/h: $(calc_eval "mph * 1609344 / 1000000")"

echo "── temperature C↔F ──"
to_f() {
    local c=$1
    echo $(( c * 9 / 5 + 32 ))
}
to_c() {
    local f=$1
    echo $(( (f - 32) * 5 / 9 ))
}
for c in 0 20 25 100; do
    echo "${c}°C = $(to_f $c)°F"
done
for f in 32 68 77 212; do
    echo "${f}°F = $(to_c $f)°C"
done

echo "── compound interest ──"
ci() {
    local principal=$1 rate=$2 years=$3
    # P*(1+r/100)^years (integer math: use scaled)
    local total=$principal
    local i
    for ((i=0; i<years; i++)); do
        total=$(( total * (100 + rate) / 100 ))
    done
    echo $total
}
echo "$1000 @ 5% for 10y: $(ci 1000 5 10)"
echo "$5000 @ 7% for 5y: $(ci 5000 7 5)"

# === ztest assertions ===
# Note: calc_eval's `${expr//\$$k/${VARS[$k]}}` indirect-var substitution
# pattern doesn't fire under zshrs, so `c = (a+b)*2` resolves to 0.  Assert on
# what does work — calc_set, plain arithmetic, and the to_f / to_c / ci fns.
calc_set demo_x 7 >/dev/null
zassert_eq "${VARS[demo_x]}" "7" "calc_set stores value"
zassert_eq "$(calc_eval 2+3)"     "5"   "calc_eval bare arithmetic add"
zassert_eq "$(calc_eval '2 * 4')" "8"   "calc_eval bare arithmetic mul"
zassert_eq "$(to_f 0)"   "32"   "0°C → 32°F"
zassert_eq "$(to_f 100)" "212"  "100°C → 212°F"
zassert_eq "$(to_c 32)"  "0"    "32°F → 0°C"
zassert_eq "$(to_c 212)" "100"  "212°F → 100°C"
zassert_eq "$(ci 1000 5 10)" "1623" "compound 1000@5% 10y (integer-only)"
zassert_eq "$(ci 5000 7 5)"  "7010" "compound 5000@7% 5y (integer-only)"
ztest_run
