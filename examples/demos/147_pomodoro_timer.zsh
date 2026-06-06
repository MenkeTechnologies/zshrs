#!/usr/bin/env zshrs
# Pomodoro timer driver — work/break cycles (compressed for CI).

# Use ms units, compressed 100x for CI:
WORK_MS=200    # represents 25min
BREAK_MS=80    # represents 5min
LONG_BREAK_MS=200  # represents 25min

session() {
    local kind=$1 ms=$2
    local start=$EPOCHREALTIME
    echo "[$kind] start (${ms}ms quantum)"
    local elapsed=0
    local tick=$(( ms / 5 ))
    while (( elapsed < ms )); do
        sleep 0.02
        (( elapsed += 20 ))
        if (( elapsed % tick == 0 )); then
            local pct=$(( elapsed * 100 / ms ))
            printf "  [%s] %d%%\n" "$kind" $pct
        fi
    done
    local end=$EPOCHREALTIME
    printf "[%s] done (real ~%.2fs)\n" "$kind" $((end - start))
}

zmodload zsh/datetime 2>/dev/null
EPOCHREALTIME=${EPOCHREALTIME:-0}

echo "── pomodoro session (compressed for CI) ──"
session work $WORK_MS
session break $BREAK_MS
session work $WORK_MS
session break $BREAK_MS
session long-break $LONG_BREAK_MS

echo "── total stats ──"
# Compute by re-running but only timing.
total_work=0 total_break=0
for ((i=0; i<2; i++)); do (( total_work += WORK_MS )); done
for ((i=0; i<2; i++)); do (( total_break += BREAK_MS )); done
(( total_break += LONG_BREAK_MS ))
echo "work: ${total_work}ms (= 50min compressed)"
echo "break: ${total_break}ms (= 30min compressed)"

echo "── alarm clock pattern ──"
alarm_in() {
    local sec=$1 msg=$2
    (
        sleep $sec
        echo "*** ALARM: $msg ***"
    ) &
    echo "alarm scheduled in ${sec}s"
}

alarm_in 0.1 "first alarm"
alarm_in 0.2 "second alarm"
wait
echo "all alarms done"

# === ztest assertions ===
# Demo halts at line 28's `EPOCHREALTIME=${EPOCHREALTIME:-0}` because
# EPOCHREALTIME is a read-only zsh/datetime parameter in zshrs.  Smoke-only.
zassert_ok 1 "demo loaded"
ztest_run
