#!/usr/bin/env zshrs
# Calendar — month grid, year overview, holiday markers.

# Day names.
DAYS=(Mo Tu We Th Fr Sa Su)
MONTHS=(January February March April May June
        July August September October November December)

# Days in month (0-indexed, 1=Feb leap adjust).
days_in_month() {
    local m=$1 y=$2
    local -a dim
    dim=(31 28 31 30 31 30 31 31 30 31 30 31)
    if (( m == 2 )); then
        if (( y % 400 == 0 )); then
            echo 29
        elif (( y % 100 == 0 )); then
            echo 28
        elif (( y % 4 == 0 )); then
            echo 29
        else
            echo 28
        fi
    else
        echo ${dim[m]}
    fi
}

# Zeller's congruence: day of week (0=Mon, 6=Sun).
day_of_week() {
    local d=$1 m=$2 y=$3
    if (( m < 3 )); then
        (( m += 12 ))
        (( y-- ))
    fi
    local k=$(( y % 100 ))
    local j=$(( y / 100 ))
    local h=$(( (d + (13 * (m + 1)) / 5 + k + k/4 + j/4 + 5*j) % 7 ))
    # Zeller: 0=Sat. Map to 0=Mon.
    echo $(( (h + 5) % 7 ))
}

# Print month.
print_month() {
    local m=$1 y=$2 width=20
    local mname="${MONTHS[m]}"
    local title="$mname $y"
    local pad=$(( (width - ${#title}) / 2 ))
    local sp=""
    local i
    for ((i=0; i<pad; i++)); do sp+=" "; done
    echo "${sp}${title}"
    # Header.
    printf "  "
    for d in "${DAYS[@]}"; do
        printf "%2s " "$d"
    done
    echo
    local dim=$(days_in_month $m $y)
    local first_dow=$(day_of_week 1 $m $y)
    # Pad.
    local col=0
    printf "  "
    for ((i=0; i<first_dow; i++)); do
        printf "   "
        (( col++ ))
    done
    for ((i=1; i<=dim; i++)); do
        printf "%2d " $i
        (( col++ ))
        if (( col == 7 )); then
            echo
            printf "  "
            col=0
        fi
    done
    if (( col != 0 )); then echo; fi
    echo
}

# Print year (4 months per row).
print_year() {
    local y=$1
    local quarter
    for quarter in 1 4 7 10; do
        # Three months per row.
        local q m
        for q in 0 1 2; do
            m=$(( quarter + q ))
            (( m > 12 )) && break
            local mname="${MONTHS[m]}"
            printf "%-25s" "$mname"
        done
        echo
        # Header row.
        for q in 0 1 2; do
            m=$(( quarter + q ))
            (( m > 12 )) && break
            printf " "
            for d in "${DAYS[@]}"; do printf "%2s " "$d"; done
            printf " "
        done
        echo
        # Compute first DOW + dim for each.
        local -a first_dows dims
        first_dows=()
        dims=()
        for q in 0 1 2; do
            m=$(( quarter + q ))
            (( m > 12 )) && break
            first_dows+=( $(day_of_week 1 $m $y) )
            dims+=( $(days_in_month $m $y) )
        done
        local -a positions
        for q in 0 1 2; do positions[q+1]=1; done
        local row
        for row in 1 2 3 4 5 6; do
            local printed_any=0
            for q in 0 1 2; do
                m=$(( quarter + q ))
                (( m > 12 )) && break
                local first_dow=${first_dows[q+1]}
                local dim=${dims[q+1]}
                local pos=${positions[q+1]}
                printf " "
                local col
                for col in 0 1 2 3 4 5 6; do
                    if (( row == 1 && col < first_dow )); then
                        printf "   "
                    elif (( pos > dim )); then
                        printf "   "
                    else
                        printf "%2d " $pos
                        (( pos++ ))
                        printed_any=1
                    fi
                done
                positions[q+1]=$pos
                printf " "
            done
            echo
            if (( ! printed_any )); then break; fi
        done
        echo
    done
}

echo "── single month: May 2026 ──"
print_month 5 2026

echo "── December 2024 ──"
print_month 12 2024

echo "── February leap (2024) ──"
print_month 2 2024

echo "── February non-leap (2025) ──"
print_month 2 2025

echo "── day-of-week tests ──"
dates=(
    "1 1 2024:Jan 1 2024 should be Mon"
    "1 1 2000:Jan 1 2000 should be Sat"
    "29 5 2026:May 29 2026 should be Fri"
    "25 12 2026:Dec 25 2026 should be Fri"
    "4 7 1776:Jul 4 1776 should be Thu"
    "20 7 1969:Jul 20 1969 should be Sun"
)
for entry in "${dates[@]}"; do
    info="${entry#*:}"
    set -- ${=entry%:*}
    dow=$(day_of_week $1 $2 $3)
    echo "  $info  → DOW=$dow (${DAYS[dow+1]})"
done

echo
echo "── leap year test ──"
echo "  rule: div by 4, NOT div by 100, OR div by 400"
for y in 1900 2000 2024 2100 2400 2025 2026; do
    feb=$(days_in_month 2 $y)
    leap="non-leap"
    (( feb == 29 )) && leap="LEAP"
    printf "  %d: Feb has %d days (%s)\n" $y $feb "$leap"
done

echo
echo "── month lengths ──"
for m in {1..12}; do
    d=$(days_in_month $m 2026)
    printf "  %-10s %2d days\n" "${MONTHS[m]}" $d
done

# === ztest assertions ===
# Leap years.
zassert_eq "$(days_in_month 2 2024)" "29"  "2024 is leap (div 4, not 100)"
zassert_eq "$(days_in_month 2 2025)" "28"  "2025 non-leap"
zassert_eq "$(days_in_month 2 2000)" "29"  "2000 is leap (div 400)"
zassert_eq "$(days_in_month 2 2100)" "28"  "2100 non-leap (div 100, not 400)"
zassert_eq "$(days_in_month 2 1900)" "28"  "1900 non-leap"
# Standard months.
zassert_eq "$(days_in_month 1  2026)" "31" "January"
zassert_eq "$(days_in_month 4  2026)" "30" "April"
zassert_eq "$(days_in_month 12 2026)" "31" "December"
# Day-of-week (0=Mon based on Zeller mapping).
zassert_eq "$(day_of_week 1 1 2024)" "0"   "Jan 1 2024 = Monday"
zassert_eq "$(day_of_week 1 1 2000)" "5"   "Jan 1 2000 = Saturday (Zeller)"
zassert_eq "${MONTHS[5]}" "May"            "MONTHS[5]"
zassert_eq "${DAYS[1]}"   "Mo"             "DAYS[1]"
ztest_run
