#!/usr/bin/env zshrs
# Append-only event log with filter + aggregate.

zmodload zsh/datetime 2>/dev/null

typeset -a LOG

log_event() {
    local level=$1
    shift
    local ts=$EPOCHSECONDS
    LOG+=("$ts|$level|$*")
}

log_filter_level() {
    local lvl=$1
    for entry in "${LOG[@]}"; do
        local entry_lvl=${${entry#*|}%%|*}
        if [[ $entry_lvl == $lvl ]]; then
            echo "$entry"
        fi
    done
}

log_count_by_level() {
    local -A counts
    for entry in "${LOG[@]}"; do
        local lvl=${${entry#*|}%%|*}
        counts[$lvl]=$(( ${counts[$lvl]:-0} + 1 ))
    done
    for k in ${(ko)counts}; do
        printf "  %-8s %d\n" $k ${counts[$k]}
    done
}

log_recent() {
    local n=${1:-5}
    local start=$(( ${#LOG[@]} - n + 1 ))
    (( start < 1 )) && start=1
    for ((i = start; i <= ${#LOG[@]}; i++)); do
        echo "${LOG[i]}"
    done
}

echo "── generate events ──"
log_event INFO "system started"
log_event DEBUG "config loaded"
log_event INFO "listening on port 8080"
log_event WARN "low disk space (12%)"
log_event ERROR "db connection failed"
log_event INFO "retrying connection"
log_event ERROR "db retry failed"
log_event WARN "switching to read-only mode"
log_event INFO "fallback active"
log_event DEBUG "cache hit ratio: 0.85"

echo "── total events: ${#LOG[@]} ──"

echo "── filter ERROR ──"
log_filter_level ERROR

echo "── filter WARN ──"
log_filter_level WARN

echo "── counts by level ──"
log_count_by_level

echo "── last 5 events ──"
log_recent 5

echo "── chronological sort (already in order) ──"
log_recent ${#LOG[@]} | head -3
echo "..."
log_recent 3

# === ztest assertions ===
zassert_eq "${#LOG[@]}" "10" "10 events logged"
# filter
err_out=$(log_filter_level ERROR)
zassert_eq "$(echo "$err_out" | wc -l | tr -d ' ')" "2" "2 ERROR events"
zassert_contains "$err_out" "db connection failed" "ERROR contains db connection failed"
zassert_contains "$err_out" "db retry failed"      "ERROR contains db retry failed"
# count by level
cnt=$(log_count_by_level)
zassert_contains "$cnt" "INFO     4"  "count INFO 4"
zassert_contains "$cnt" "DEBUG    2"  "count DEBUG 2"
zassert_contains "$cnt" "ERROR    2"  "count ERROR 2"
zassert_contains "$cnt" "WARN     2"  "count WARN 2"
# recent slicing
zassert_eq "$(log_recent 5 | wc -l | tr -d ' ')" "5" "last 5 events"
zassert_contains "$(log_recent 1)" "cache hit ratio" "log_recent 1 is latest"
ztest_run
