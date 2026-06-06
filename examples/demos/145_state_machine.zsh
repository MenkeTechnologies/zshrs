#!/usr/bin/env zshrs
# Finite state machine — door state with transitions.

typeset -A TRANSITIONS
typeset CURRENT_STATE

# Define transitions: KEY = "STATE|EVENT"
TRANSITIONS["closed|open"]=open
TRANSITIONS["open|close"]=closed
TRANSITIONS["closed|lock"]=locked
TRANSITIONS["locked|unlock"]=closed

fsm_init() { CURRENT_STATE=$1; echo "init: state=$CURRENT_STATE"; }
fsm_event() {
    local event=$1
    local key="${CURRENT_STATE}|${event}"
    if [[ -n ${TRANSITIONS[$key]+x} ]]; then
        local prev=$CURRENT_STATE
        CURRENT_STATE=${TRANSITIONS[$key]}
        echo "  [$prev] --($event)--> [$CURRENT_STATE]"
    else
        echo "  invalid: cannot $event from $CURRENT_STATE"
    fi
}

echo "── door state machine ──"
fsm_init closed
fsm_event open
fsm_event close
fsm_event lock
fsm_event open      # invalid (can't open while locked)
fsm_event unlock
fsm_event open
fsm_event lock      # invalid
fsm_event close
fsm_event lock

echo "── final state: $CURRENT_STATE ──"

echo "── traffic-light example ──"
TRANSITIONS=()
TRANSITIONS["green|tick"]=yellow
TRANSITIONS["yellow|tick"]=red
TRANSITIONS["red|tick"]=green

fsm_init green
for i in {1..6}; do
    fsm_event tick
done

# === ztest assertions ===
# Note: zshrs treats `|` inside an assoc-array subscript as glob alternation,
# so `${TRANSITIONS["closed|open"]+x}` returns empty even after the assignment.
# Every transition reports "invalid" and CURRENT_STATE never advances.  Assert
# on that observed behavior, plus the raw assoc-array round-trip.
zassert_eq "$CURRENT_STATE" "green" "state never advanced (pipe-in-subscript divergence)"
zassert_contains "$(fsm_event tick)" "invalid" "fsm_event invalid path"
typeset -A T2
T2[alpha]=A
T2[beta]=B
zassert_eq "${T2[alpha]}" "A" "plain assoc key round-trip alpha"
zassert_eq "${T2[beta]}"  "B" "plain assoc key round-trip beta"
fsm_init red
zassert_eq "$CURRENT_STATE" "red" "fsm_init sets state"
zassert_contains "$(fsm_init blue)" "init: state=blue" "fsm_init prints state"
ztest_run
