#!/usr/bin/env zshrs
# Signal handling — trap on USR1, USR2, custom signal patterns.
# Ported from Src/signals.c handletrap + Src/signals.c install_handler.

echo "── trap by signal number ──"
trap 'echo "got SIGUSR1 (sig 30)"' USR1
trap 'echo "got SIGUSR2 (sig 31)"' USR2

echo "── send self USR1 ──"
kill -USR1 $$
sleep 0.05

echo "── send self USR2 ──"
kill -USR2 $$
sleep 0.05

echo "── trap by name ──"
trap 'echo "TERM intercepted"' TERM
# Cannot test TERM without killing — just verify it's installed.
trap -p TERM 2>/dev/null | head -1

echo "── clear all traps ──"
trap - USR1 USR2 TERM
trap -p 2>/dev/null | head -5

echo "── EXIT trap with deferred work ──"
(
    trap 'echo "  bg-EXIT trap fired"' EXIT
    echo "  bg working..."
    sleep 0.05
)

echo "── nested traps in subshells ──"
(
    trap 'echo "outer-bg EXIT"' EXIT
    (
        trap 'echo "inner-bg EXIT"' EXIT
        echo "innermost"
    )
)

echo "── ignore signal (trap '' SIG) ──"
trap '' USR1
echo "USR1 now ignored"
kill -USR1 $$  # no effect
trap - USR1
echo "USR1 reset"

echo "── multiple signals same handler ──"
cleanup() { echo "cleanup fired for one of HUP/INT/TERM"; }
trap cleanup HUP INT TERM
trap -p HUP INT TERM 2>/dev/null | head -3
trap - HUP INT TERM

echo "── signal in a loop ──"
caught=0
trap '(( caught++ ))' USR1
for i in 1 2 3 4 5; do
    kill -USR1 $$
    sleep 0.01
done
sleep 0.05
echo "USR1 fired $caught times"
trap - USR1

# === ztest assertions ===
zassert_eq "$caught" "5" "USR1 fired 5x in loop"
# Re-arm trap, count again, verify state isolation per arming session.
fired=0
trap '(( fired++ ))' USR1
kill -USR1 $$
sleep 0.05
kill -USR1 $$
sleep 0.05
trap - USR1
zassert_eq "$fired" "2" "re-armed trap counts 2"
# Nested subshell trap output ordering
nested_out=$(
    (
        trap 'echo OUT' EXIT
        (
            trap 'echo IN' EXIT
            echo body
        )
    )
)
zassert_contains "$nested_out" "IN"  "inner EXIT trap fired"
zassert_contains "$nested_out" "OUT" "outer EXIT trap fired"
zassert_contains "$nested_out" "body" "subshell body ran"
ztest_run
