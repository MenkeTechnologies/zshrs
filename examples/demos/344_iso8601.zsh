#!/usr/bin/env zshrs
# ISO 8601 — comprehensive date/time/duration parser.

# Formats:
#   2026-05-29           date
#   2026-05-29T14:30:00  datetime
#   2026-05-29T14:30:00Z UTC
#   2026-05-29T14:30:00+05:30  offset
#   2026-W22             week date
#   2026-149             ordinal date
#   PT2H30M              duration

parse_iso8601() {
    local s=$1
    typeset -gA I
    I=()
    I[input]="$s"

    # Duration: starts with P.
    if [[ $s == P* ]]; then
        I[type]="duration"
        local dur="${s#P}"
        # Date part: nYnMnD before T.
        local date_part="${dur%%T*}"
        local time_part=""
        if [[ $dur == *T* ]]; then
            time_part="${dur#*T}"
        fi
        I[dur_date]=$date_part
        I[dur_time]=$time_part

        # Parse components.
        local i
        local n=""
        local field=""
        for ((i=1; i<=${#date_part}; i++)); do
            local c="${date_part[i]}"
            case $c in
                [0-9]) n+="$c" ;;
                Y) I[years]=$n; n="" ;;
                M) I[months]=$n; n="" ;;
                W) I[weeks]=$n; n="" ;;
                D) I[days]=$n; n="" ;;
            esac
        done
        for ((i=1; i<=${#time_part}; i++)); do
            local c="${time_part[i]}"
            case $c in
                [0-9]) n+="$c" ;;
                H) I[hours]=$n; n="" ;;
                M) I[minutes]=$n; n="" ;;
                S) I[seconds]=$n; n="" ;;
            esac
        done
        return
    fi

    # Week date: YYYY-Www-D or YYYY-Www
    if [[ $s == *W* && $s != *T* ]]; then
        I[type]="week"
        I[year]="${s%%-*}"
        local rest="${s#*-W}"
        I[week]="${rest%%-*}"
        if [[ $rest == *-* ]]; then
            I[weekday]="${rest#*-}"
        else
            I[weekday]="1"
        fi
        return
    fi

    # Ordinal date: YYYY-DDD
    if [[ $s == [0-9]([0-9])(#c3)-[0-9](#c3) ]]; then
        I[type]="ordinal"
        I[year]="${s%%-*}"
        I[day_of_year]="${s#*-}"
        return
    fi

    # Date or datetime.
    if [[ $s == *T* ]]; then
        I[type]="datetime"
        local date_part="${s%%T*}"
        local time_full="${s#*T}"
        I[date]=$date_part
        I[year]="${date_part%%-*}"
        local d_rest="${date_part#*-}"
        I[month]="${d_rest%%-*}"
        I[day]="${d_rest#*-}"

        # Time may have timezone.
        local time_only="" tz=""
        if [[ $time_full == *Z ]]; then
            time_only="${time_full%Z}"
            tz="Z"
        elif [[ $time_full == *+* ]]; then
            time_only="${time_full%+*}"
            tz="+${time_full#*+}"
        elif [[ $time_full == *-* ]]; then
            # Could be -HH:MM (negative offset).
            # Naively: check if dash is after T position 3.
            time_only="${time_full%-*}"
            tz="-${time_full##*-}"
        else
            time_only="$time_full"
        fi
        I[time]=$time_only
        I[hour]="${time_only%%:*}"
        local t_rest="${time_only#*:}"
        I[minute]="${t_rest%%:*}"
        if [[ $t_rest == *:* ]]; then
            I[second]="${t_rest#*:}"
        else
            I[second]="00"
        fi
        I[zone]=$tz
        return
    fi

    # Plain date.
    I[type]="date"
    I[year]="${s%%-*}"
    local rest="${s#*-}"
    I[month]="${rest%%-*}"
    I[day]="${rest#*-}"
}

echo "── parse various ISO 8601 forms ──"
samples=(
    "2026-05-29"
    "2026-05-29T14:30:00"
    "2026-05-29T14:30:00Z"
    "2026-05-29T14:30:00+05:30"
    "2026-05-29T14:30:00-08:00"
    "2026-W22"
    "2026-W22-5"
    "2026-149"
    "PT2H30M"
    "P3Y6M4DT12H30M5S"
    "PT15M"
    "P1Y"
)
for s in "${samples[@]}"; do
    parse_iso8601 "$s"
    echo
    echo "  '$s' (type: ${I[type]})"
    case ${I[type]} in
        date|datetime)
            echo "    year=${I[year]} month=${I[month]} day=${I[day]}"
            if [[ ${I[type]} == datetime ]]; then
                echo "    hour=${I[hour]} minute=${I[minute]} second=${I[second]} zone=${I[zone]:-none}"
            fi
            ;;
        week)
            echo "    year=${I[year]} week=${I[week]} weekday=${I[weekday]}"
            ;;
        ordinal)
            echo "    year=${I[year]} day-of-year=${I[day_of_year]}"
            ;;
        duration)
            for k in years months weeks days hours minutes seconds; do
                v=${I[$k]:-}
                [[ -n $v ]] && echo "    $k = $v"
            done
            ;;
    esac
done

echo
echo "── duration to seconds ──"
duration_to_seconds() {
    parse_iso8601 "$1"
    local years=${I[years]:-0}
    local months=${I[months]:-0}
    local days=${I[days]:-0}
    local hours=${I[hours]:-0}
    local minutes=${I[minutes]:-0}
    local seconds=${I[seconds]:-0}
    # Rough: year=365.25 days, month=30.44 days.
    local secs=$(( years * 31557600 + months * 2629800 + days * 86400 + \
                   hours * 3600 + minutes * 60 + seconds ))
    echo $secs
}

for d in PT30S PT15M PT1H P1D P7D P1M P1Y PT1H30M; do
    s=$(duration_to_seconds "$d")
    printf "  %-12s = %10d seconds\n" "$d" "$s"
done

echo
echo "── date arithmetic ──"
add_seconds_to_date() {
    parse_iso8601 "$1"
    local secs_add=$2
    local total_secs=$(( ${I[hour]} * 3600 + ${I[minute]} * 60 + ${I[second]} + secs_add ))
    local new_h=$(( total_secs / 3600 % 24 ))
    local new_m=$(( total_secs / 60 % 60 ))
    local new_s=$(( total_secs % 60 ))
    local day_overflow=$(( total_secs / 86400 ))
    local new_day=$(( ${I[day]} + day_overflow ))
    printf "%s-%02d-%02d T %02d:%02d:%02d\n" "${I[year]}" "${I[month]}" "$new_day" "$new_h" "$new_m" "$new_s"
}

base="2026-05-29T14:30:00"
echo "  base: $base"
echo "  +1h30m:    $(add_seconds_to_date $base 5400)"
echo "  +6h:       $(add_seconds_to_date $base 21600)"
echo "  +1d:       $(add_seconds_to_date $base 86400)"

echo
echo "── format validation ──"
strict_check() {
    local s=$1
    # YYYY-MM-DD must have YYYY 4 digits, MM 2, DD 2.
    if [[ $s == [0-9](#c4)-[0-9](#c2)-[0-9](#c2) ]]; then
        return 0
    fi
    return 1
}

dates=(
    "2026-05-29"
    "26-05-29"        # 2 digit year
    "2026-5-29"       # 1 digit month
    "2026-05-9"       # 1 digit day
    "2026/05/29"      # wrong separator
    "20260529"        # no separators
)
for d in "${dates[@]}"; do
    if strict_check "$d"; then
        echo "  ✓ $d (strict ISO 8601)"
    else
        echo "  ✗ $d (loose / non-strict)"
    fi
done

# === ztest assertions ===
parse_iso8601 "2026-05-29"
zassert_eq "${I[type]}"  "date"  "plain date type"
zassert_eq "${I[year]}"  "2026"  "year"
zassert_eq "${I[month]}" "05"    "month"
zassert_eq "${I[day]}"   "29"    "day"
parse_iso8601 "2026-05-29T14:30:00Z"
zassert_eq "${I[type]}" "datetime" "datetime type"
zassert_eq "${I[hour]}" "14"       "hour"
zassert_eq "${I[zone]}" "Z"        "UTC zone"
parse_iso8601 "P1Y"
zassert_eq "${I[type]}"  "duration" "duration type"
zassert_eq "${I[years]}" "1"        "years field"
parse_iso8601 "PT2H30M"
zassert_eq "${I[hours]}"   "2"   "duration hours"
zassert_eq "${I[minutes]}" "30"  "duration minutes"
zassert_eq "$(duration_to_seconds PT1H30M)" 5400 "1h30m = 5400s"
zassert_eq "$(duration_to_seconds PT30S)"   30   "30s"
zassert_eq "$(duration_to_seconds P1D)"     86400 "1 day"
ztest_run
