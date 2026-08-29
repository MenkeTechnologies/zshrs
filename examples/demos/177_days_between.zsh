#!/usr/bin/env zshrs
# Days between two dates — via zsh/datetime strftime + epoch math.

zmodload zsh/datetime 2>/dev/null

# Parse YYYY-MM-DD into Y,M,D ints.
parse_date() {
    local d=$1
    local y=${d[1,4]}
    local m=${d[6,7]}
    local dd=${d[9,10]}
    echo "$y $m $dd"
}

is_leap() {
    local y=$1
    if (( y % 400 == 0 )); then echo 1
    elif (( y % 100 == 0 )); then echo 0
    elif (( y % 4 == 0 )); then echo 1
    else echo 0
    fi
}

days_in_month() {
    local m=$1 y=$2
    case $m in
        1|3|5|7|8|10|12) echo 31 ;;
        4|6|9|11) echo 30 ;;
        2) (( $(is_leap $y) )) && echo 29 || echo 28 ;;
    esac
}

# Days since year 1900 (avoids 2000-iter year loop for distant dates).
# Inline leap test + days-in-month table for speed (no fn calls).
days_since_1900() {
    local y=$1 m=$2 d=$3
    local total=0 yr mo leap dim
    local -a months=(31 28 31 30 31 30 31 31 30 31 30 31)
    for ((yr=1900; yr<y; yr++)); do
        if (( yr % 400 == 0 )); then
            (( total += 366 ))
        elif (( yr % 100 == 0 )); then
            (( total += 365 ))
        elif (( yr % 4 == 0 )); then
            (( total += 366 ))
        else
            (( total += 365 ))
        fi
    done
    if (( y % 400 == 0 )); then leap=1
    elif (( y % 100 == 0 )); then leap=0
    elif (( y % 4 == 0 )); then leap=1
    else leap=0
    fi
    for ((mo=1; mo<m; mo++)); do
        dim=${months[mo]}
        (( mo == 2 && leap )) && (( dim++ ))
        (( total += dim ))
    done
    (( total += d - 1 ))
    echo $total
}

days_between() {
    local d1=$1 d2=$2
    local p1=( $(parse_date $d1) )
    local p2=( $(parse_date $d2) )
    local e1=$(days_since_1900 ${p1[1]} ${p1[2]} ${p1[3]})
    local e2=$(days_since_1900 ${p2[1]} ${p2[2]} ${p2[3]})
    local diff=$(( e2 - e1 ))
    (( diff < 0 )) && diff=$(( -diff ))
    echo $diff
}

echo "── basic dates ──"
echo "2026-01-01 to 2026-01-31: $(days_between 2026-01-01 2026-01-31)"
echo "2025-01-01 to 2026-01-01: $(days_between 2025-01-01 2026-01-01)"
echo "2024-02-29 to 2025-02-28: $(days_between 2024-02-29 2025-02-28)"

echo "── leap year check ──"
for y in 2020 2021 2022 2023 2024 2025 2100 2400; do
    if (( $(is_leap $y) )); then
        echo "$y: leap"
    else
        echo "$y: not leap"
    fi
done

echo "── days in each month of 2024 ──"
for m in 1 2 3 4 5 6 7 8 9 10 11 12; do
    printf "%4d-%02d: %d days\n" 2024 $m $(days_in_month $m 2024)
done

echo "── historical events ──"
echo "moon landing → today (approx):"
echo "  1969-07-20 to 2026-05-29: $(days_between 1969-07-20 2026-05-29) days"
echo "Y2K to today:"
echo "  2000-01-01 to 2026-05-29: $(days_between 2000-01-01 2026-05-29) days"

# === ztest assertions ===
# Leap year detector is purely arithmetic — assert against rule.
zassert_eq "$(is_leap 2024)" 1               "2024 is leap"
zassert_eq "$(is_leap 2025)" 0               "2025 not leap"
zassert_eq "$(is_leap 2100)" 0               "2100 not leap (div by 100, not 400)"
zassert_eq "$(is_leap 2400)" 1               "2400 is leap (div by 400)"
# days_in_month.
zassert_eq "$(days_in_month 1 2024)" 31      "Jan 2024 = 31"
zassert_eq "$(days_in_month 2 2024)" 29      "Feb 2024 = 29 (leap)"
zassert_eq "$(days_in_month 2 2025)" 28      "Feb 2025 = 28"
zassert_eq "$(days_in_month 4 2024)" 30      "Apr 2024 = 30"
# parse_date.
zassert_eq "$(parse_date 2026-05-29)" "2026 05 29" "parse_date splits"
zassert_eq "$(days_between 2026-01-01 2026-01-31)" 30  "Jan 1 → Jan 31 is 30 days"
zassert_eq "$(days_between 2025-01-01 2026-01-01)" 365 "a common year is 365 days"
zassert_eq "$(days_between 2024-01-01 2025-01-01)" 366 "a leap year is 366 days"
zassert_eq "$(days_between 2026-01-31 2026-01-01)" 30  "argument order does not matter"
ztest_run
