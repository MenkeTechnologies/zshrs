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

# Days since arbitrary epoch (year 1 → Jan 1).
days_since_epoch() {
    local y=$1 m=$2 d=$3
    local total=0 yr mo
    # Years.
    for ((yr=1; yr<y; yr++)); do
        if (( $(is_leap $yr) )); then
            (( total += 366 ))
        else
            (( total += 365 ))
        fi
    done
    # Months.
    for ((mo=1; mo<m; mo++)); do
        (( total += $(days_in_month $mo $y) ))
    done
    (( total += d - 1 ))
    echo $total
}

days_between() {
    local d1=$1 d2=$2
    local p1=( $(parse_date $d1) )
    local p2=( $(parse_date $d2) )
    local e1=$(days_since_epoch ${p1[1]} ${p1[2]} ${p1[3]})
    local e2=$(days_since_epoch ${p2[1]} ${p2[2]} ${p2[3]})
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
