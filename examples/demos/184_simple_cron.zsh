#!/usr/bin/env zshrs
# Cron-like scheduler simulation — fixed time table run.

zmodload zsh/datetime 2>/dev/null

typeset -A SCHEDULE

schedule() {
    local cron=$1
    local task=$2
    SCHEDULE[$cron]=$task
}

# cron-like form: "MIN HOUR DAY MONTH WEEKDAY"
# * is wildcard.
matches_cron() {
    local cron=$1 min=$2 hour=$3 day=$4 month=$5 wday=$6
    local -a parts=( ${(s/ /)cron} )
    local c v
    local -a targets=($min $hour $day $month $wday)
    for i in 1 2 3 4 5; do
        c=${parts[i]}
        v=${targets[i]}
        if [[ $c == "*" ]]; then continue; fi
        if [[ $c == */* ]]; then
            local step=${c#*/}
            (( v % step != 0 )) && return 1
            continue
        fi
        if [[ $c != $v ]]; then return 1; fi
    done
    return 0
}

# Pre-schedule.
schedule "0 0 * * *"      "daily-midnight"
schedule "0 12 * * *"     "daily-noon"
schedule "30 9 * * *"     "daily-9:30"
schedule "0 0 1 * *"      "monthly-1st"
schedule "0 * * * *"      "hourly"
schedule "*/15 * * * *"   "every-15min"

echo "── schedule ──"
for k v in "${(@kv)SCHEDULE}"; do
    printf "  %-20s → %s\n" "$k" "$v"
done

echo
echo "── simulate 6 ticks across a day ──"
for tick in "0 0 1 5 1" "15 9 1 5 1" "30 9 1 5 1" "0 12 1 5 1" "45 14 1 5 1" "0 0 2 5 2"; do
    set -- $=tick
    min=$1; hour=$2; day=$3; month=$4; wday=$5
    printf "%02d:%02d on day %d (month %d, wday %d):\n" $hour $min $day $month $wday
    fired=0
    for cron in ${(k)SCHEDULE}; do
        if matches_cron "$cron" "$min" "$hour" "$day" "$month" "$wday"; then
            echo "  fired: $cron → ${SCHEDULE[$cron]}"
            (( fired++ ))
        fi
    done
    (( fired == 0 )) && echo "  (nothing fired)"
done

echo
echo "── every-15min over 1 hour ──"
for m in 0 5 15 30 45 60; do
    matches_cron "*/15 * * * *" "$m" 12 1 5 1 && echo "  ${m}min: would fire"
done
