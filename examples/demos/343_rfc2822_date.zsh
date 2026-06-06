#!/usr/bin/env zshrs
# RFC 2822 / 5322 date parser — used in email headers, HTTP, etc.

typeset -A MONTHS
MONTHS=(
    Jan 1   Feb 2   Mar 3   Apr 4   May 5   Jun 6
    Jul 7   Aug 8   Sep 9   Oct 10  Nov 11  Dec 12
)

typeset -A DAYS
DAYS=(
    Mon 1   Tue 2   Wed 3   Thu 4   Fri 5   Sat 6   Sun 7
)

# Parse "Day, DD Mon YYYY HH:MM:SS ZONE"
# Stores parsed fields in $D[*].
parse_rfc2822() {
    local date_str=$1
    typeset -gA D
    D=()
    # Strip leading whitespace.
    date_str="${date_str## }"
    # Day-of-week may be optional.
    local rest="$date_str"
    if [[ $rest == *,* ]]; then
        local dow="${rest%%,*}"
        rest="${rest#*, }"
        D[dow]=$dow
    fi
    # DD Mon YYYY HH:MM:SS ZONE
    local day="${rest%% *}"
    rest="${rest#* }"
    local mon="${rest%% *}"
    rest="${rest#* }"
    local year="${rest%% *}"
    rest="${rest#* }"
    local time="${rest%% *}"
    rest="${rest#* }"
    local zone="$rest"

    D[day]=$day
    D[month_name]=$mon
    D[month]=${MONTHS[$mon]:-?}
    D[year]=$year
    D[time]=$time
    D[zone]=$zone

    # Parse time.
    D[hour]="${time%%:*}"
    local t2="${time#*:}"
    D[minute]="${t2%%:*}"
    D[second]="${t2#*:}"
}

# Validate.
is_valid_date() {
    local mon=$1 day=$2 year=$3
    (( year < 1900 || year > 9999 )) && return 1
    (( mon < 1 || mon > 12 )) && return 1
    local -a days_in_mo
    days_in_mo=(31 28 31 30 31 30 31 31 30 31 30 31)
    # Leap year.
    if (( mon == 2 )); then
        if (( year % 400 == 0 )); then
            days_in_mo[2]=29
        elif (( year % 100 == 0 )); then
            :
        elif (( year % 4 == 0 )); then
            days_in_mo[2]=29
        fi
    fi
    (( day < 1 || day > days_in_mo[mon] )) && return 1
    return 0
}

# Convert to seconds since RFC epoch (Jan 1 1900).
rfc2822_to_epoch() {
    local mon=$1 day=$2 year=$3 hour=$4 min=$5 sec=$6 zone=$7
    local days_since_epoch=0
    local y m
    for ((y=1900; y<year; y++)); do
        if (( y % 400 == 0 )); then
            (( days_since_epoch += 366 ))
        elif (( y % 100 == 0 )); then
            (( days_since_epoch += 365 ))
        elif (( y % 4 == 0 )); then
            (( days_since_epoch += 366 ))
        else
            (( days_since_epoch += 365 ))
        fi
    done
    local -a dim
    dim=(31 28 31 30 31 30 31 31 30 31 30 31)
    if (( year % 400 == 0 )) || ( (( year % 100 != 0 )) && (( year % 4 == 0 )) ); then
        dim[2]=29
    fi
    for ((m=1; m<mon; m++)); do
        (( days_since_epoch += dim[m] ))
    done
    (( days_since_epoch += day - 1 ))
    local secs=$(( days_since_epoch * 86400 + hour * 3600 + min * 60 + sec ))
    # Zone offset.
    local zone_offset=0
    if [[ $zone == [+-]* ]]; then
        local sign=${zone[1]}
        local hhmm=${zone[2,-1]}
        local zh=${hhmm[1,2]}
        local zm=${hhmm[3,4]}
        local off=$(( zh * 3600 + zm * 60 ))
        if [[ $sign == "+" ]]; then
            (( secs -= off ))
        else
            (( secs += off ))
        fi
    elif [[ $zone == "GMT" || $zone == "UT" || $zone == "UTC" || $zone == "Z" ]]; then
        :
    fi
    echo $secs
}

echo "── parse RFC 2822 dates ──"
samples=(
    "Mon, 31 May 2026 14:30:00 +0000"
    "Tue, 01 Jan 2024 00:00:00 GMT"
    "Sat, 15 Jul 1995 11:45:23 -0500"
    "Sun, 25 Dec 2022 09:00:00 +0900"
    "Fri, 04 Jul 1776 12:00:00 +0000"
    "Wed, 19 Jan 2000 03:14:07 +0000"
    "29 Feb 2024 16:00:00 +0000"
)
for s in "${samples[@]}"; do
    parse_rfc2822 "$s"
    valid="✓"
    is_valid_date ${D[month]} ${D[day]} ${D[year]} || valid="✗"
    printf "  %-40s %s\n" "$s" "$valid"
    printf "    DOW=%-3s  day=%-2s  mon=%-3s(%2d)  year=%s  time=%s  zone=%s\n" \
        "${D[dow]:-N/A}" "${D[day]}" "${D[month_name]}" "${D[month]}" \
        "${D[year]}" "${D[time]}" "${D[zone]}"
done

echo
echo "── validation tests ──"
invalid_dates=(
    "Feb 30 2024"
    "Feb 29 2023"   # not leap
    "Apr 31 2024"
    "Jan 0 2024"
    "Dec 32 2024"
    "Mar 15 2024"   # valid
    "Feb 29 2024"   # valid leap
)
for entry in "${invalid_dates[@]}"; do
    set -- ${=entry}
    mon=${MONTHS[$1]}
    day=$2
    year=$3
    if is_valid_date $mon $day $year; then
        echo "  $entry: ✓ valid"
    else
        echo "  $entry: ✗ invalid"
    fi
done

echo
echo "── epoch conversion ──"
parse_rfc2822 "Thu, 01 Jan 1970 00:00:00 +0000"
e=$(rfc2822_to_epoch ${D[month]} ${D[day]} ${D[year]} ${D[hour]} ${D[minute]} ${D[second]} ${D[zone]})
echo "  Unix epoch (Jan 1 1970): $e (since RFC 1900)"
echo "  expected: 2208988800"

parse_rfc2822 "Thu, 29 May 2026 12:00:00 +0000"
e=$(rfc2822_to_epoch ${D[month]} ${D[day]} ${D[year]} ${D[hour]} ${D[minute]} ${D[second]} ${D[zone]})
echo "  2026-05-29 12:00: $e (since 1900)"

echo
echo "── timezone parsing ──"
zones=(
    "+0000:UTC"
    "+0500:India offset"
    "-0500:US Eastern"
    "-0800:US Pacific"
    "+0900:Japan"
    "+1200:NZ"
    "GMT:legacy GMT"
    "UT:legacy UT"
)
for z in "${zones[@]}"; do
    zone="${z%:*}"
    desc="${z#*:}"
    printf "  %-6s %s\n" "$zone" "$desc"
done

echo
echo "── month names ──"
echo "  short month → number:"
for m in Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec; do
    printf "    %s = %d\n" "$m" "${MONTHS[$m]}"
done

# === ztest assertions ===
parse_rfc2822 "Mon, 31 May 2026 14:30:00 +0000"
zassert_eq "${D[dow]}"       "Mon"   "dow parsed"
zassert_eq "${D[day]}"       "31"    "day parsed"
zassert_eq "${D[month]}"     "5"     "month resolved"
zassert_eq "${D[year]}"      "2026"  "year parsed"
zassert_eq "${D[hour]}"      "14"    "hour parsed"
zassert_eq "${D[minute]}"    "30"    "min parsed"
zassert_eq "${D[second]}"    "00"    "sec parsed"
zassert_eq "${D[zone]}"      "+0000" "zone parsed"
zassert_eq "${MONTHS[Jan]}"  1       "Jan = 1"
zassert_eq "${MONTHS[Dec]}"  12      "Dec = 12"
if is_valid_date 2 29 2024; then zassert_ok 1 "leap Feb 29 2024"
else zassert_ok 0 "should be valid"; fi
if is_valid_date 2 29 2023; then zassert_ok 0 "shouldn't be valid"
else zassert_ok 1 "non-leap Feb 29 2023 rejected"; fi
zassert_eq "$(rfc2822_to_epoch 1 1 1970 0 0 0 +0000)" 2208988800 "unix epoch from 1900"
ztest_run
