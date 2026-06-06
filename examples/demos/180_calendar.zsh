#!/usr/bin/env zshrs
# Calendar generator — print a month grid.

zmodload zsh/datetime 2>/dev/null

# Zeller's congruence for day of week (Mon=0..Sun=6 in our local code).
# Use the formula: h = (q + floor((13*(m+1))/5) + K + floor(K/4) + floor(J/4) - 2*J) mod 7
# Returns 0=Sat, 1=Sun, 2=Mon, ..., 6=Fri.
zeller() {
    local y=$1 m=$2 d=$3
    if (( m < 3 )); then (( m += 12, y-- )); fi
    local K=$(( y % 100 ))
    local J=$(( y / 100 ))
    local h=$(( (d + (13 * (m + 1)) / 5 + K + K/4 + J/4 + 5*J) % 7 ))
    echo $h
}

is_leap() {
    local y=$1
    (( y % 400 == 0 )) && { echo 1; return; }
    (( y % 100 == 0 )) && { echo 0; return; }
    (( y % 4 == 0 )) && { echo 1; return; }
    echo 0
}

days_in_month() {
    local m=$1 y=$2
    case $m in
        1|3|5|7|8|10|12) echo 31 ;;
        4|6|9|11) echo 30 ;;
        2) (( $(is_leap $y) )) && echo 29 || echo 28 ;;
    esac
}

month_names=("" January February March April May June July August September October November December)

print_calendar() {
    local y=$1 m=$2
    local name="${month_names[m]} $y"
    # Center the name over the 20-char grid.
    local pad=$(( (20 - ${#name}) / 2 ))
    local sp=""; local i
    for ((i=0; i<pad; i++)); do sp+=" "; done
    echo "${sp}${name}"
    echo "Su Mo Tu We Th Fr Sa"
    local first=$(zeller $y $m 1)
    # Zeller: 0=Sat,1=Sun,...,6=Fri. Map to Sun-first column (Sun=0).
    local col=$(( (first + 6) % 7 ))
    # col is now 0=Sun..6=Sat.
    for ((i=0; i<col; i++)); do printf "   "; done
    local n=$(days_in_month $m $y)
    for ((d=1; d<=n; d++)); do
        printf "%2d " $d
        (( col++ ))
        if (( col == 7 )); then
            echo
            col=0
        fi
    done
    (( col > 0 )) && echo
}

echo "── January 2026 ──"
print_calendar 2026 1

echo
echo "── February 2026 ──"
print_calendar 2026 2

echo
echo "── May 2026 ──"
print_calendar 2026 5

echo
echo "── February 2024 (leap year) ──"
print_calendar 2024 2

echo
echo "── February 2025 ──"
print_calendar 2025 2

echo
echo "── December 2026 ──"
print_calendar 2026 12

# === ztest assertions ===
# Leap-year detector.
zassert_eq "$(is_leap 2024)" 1   "2024 is leap"
zassert_eq "$(is_leap 2023)" 0   "2023 not leap"
zassert_eq "$(is_leap 2400)" 1   "2400 is leap (400 rule)"
zassert_eq "$(is_leap 2100)" 0   "2100 not leap (100 not 400)"
# days_in_month.
zassert_eq "$(days_in_month 2 2024)" 29 "Feb 2024 = 29"
zassert_eq "$(days_in_month 2 2025)" 28 "Feb 2025 = 28"
zassert_eq "$(days_in_month 1 2026)" 31 "Jan 2026 = 31"
zassert_eq "$(days_in_month 4 2026)" 30 "Apr 2026 = 30"
# Zeller returns a 0..6 value.
z=$(zeller 2026 1 1)
zassert_ge "$z" 0  "zeller >= 0"
zassert_le "$z" 6  "zeller <= 6"
# print_calendar emits a header row with days.
cal_jan="$(print_calendar 2026 1)"
zassert_contains "$cal_jan" "Su Mo Tu We Th Fr Sa"  "weekday header"
zassert_contains "$cal_jan" "31"                    "January has day 31"
ztest_run
