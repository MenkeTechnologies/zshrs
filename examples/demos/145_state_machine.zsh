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
