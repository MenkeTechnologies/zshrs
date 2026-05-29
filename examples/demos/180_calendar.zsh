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
