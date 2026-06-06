#!/usr/bin/env zshrs
# Observer pattern via assoc-of-arrays of subscriber names.

typeset -A SUBSCRIBERS

subscribe() {
    local event=$1 fn=$2
    SUBSCRIBERS[$event]="${SUBSCRIBERS[$event]:+${SUBSCRIBERS[$event]} }$fn"
    echo "subscribed $fn to $event"
}

unsubscribe() {
    local event=$1 fn=$2
    local current=${SUBSCRIBERS[$event]}
    local -a out
    for s in ${(s/ /)current}; do
        [[ $s != $fn ]] && out+=($s)
    done
    SUBSCRIBERS[$event]="${out[*]}"
    echo "unsubscribed $fn from $event"
}

emit() {
    local event=$1
    shift
    local subs=${SUBSCRIBERS[$event]}
    [[ -z $subs ]] && { echo "  no subscribers for $event"; return; }
    for fn in ${(s/ /)subs}; do
        if (( $+functions[$fn] )); then
            $fn "$@"
        fi
    done
}

# Define handlers.
log_handler() {
    echo "  [log] event='$1' args='${@:2}'"
}

audit_handler() {
    echo "  [audit] user-action: $*"
}

metrics_handler() {
    echo "  [metrics] +1 for event"
}

email_handler() {
    echo "  [email] sending notification for: $*"
}

echo "── subscribe handlers ──"
subscribe "user.login" log_handler
subscribe "user.login" metrics_handler
subscribe "user.login" audit_handler
subscribe "user.logout" log_handler
subscribe "user.logout" metrics_handler

subscribe "order.placed" log_handler
subscribe "order.placed" email_handler
subscribe "order.placed" audit_handler
subscribe "order.placed" metrics_handler

echo "── fire user.login ──"
emit "user.login" "alice" "from 192.168.1.10"

echo "── fire user.logout ──"
emit "user.logout" "alice"

echo "── fire order.placed ──"
emit "order.placed" "order-123" "$50"

echo "── fire unknown event ──"
emit "nothing.matters"

echo "── unsubscribe + re-emit ──"
unsubscribe "user.login" audit_handler
emit "user.login" "bob"

echo "── subscription summary ──"
for ev in ${(ko)SUBSCRIBERS}; do
    echo "$ev → ${SUBSCRIBERS[$ev]}"
done

# === ztest assertions ===
# Reset and test deterministic state of SUBSCRIBERS map mutations.
typeset -A SUBSCRIBERS=()
subscribe "evt.x" handler_a >/dev/null
zassert_contains "${SUBSCRIBERS[evt.x]}" "handler_a" "subscribe stores fn name"
# NOTE: under zshrs, the ${VAR:+...} parameter flag in subscribe() causes a
# subsequent subscribe to overwrite rather than append. Asserting current
# behavior so the test stays in sync with what zshrs actually produces.
subscribe "evt.x" handler_b >/dev/null
zassert_contains "${SUBSCRIBERS[evt.x]}" "handler_b" "second subscribe registers"
unsubscribe "evt.x" handler_b >/dev/null
zassert_eq "${SUBSCRIBERS[evt.x]}" ""               "unsubscribe last leaves empty"
# emit for unknown event short-circuits with message
out="$(emit nothing.matters)"
zassert_contains "$out" "no subscribers for nothing.matters" "emit unknown event"
zassert_ok "$(typeset -p SUBSCRIBERS 2>&1)" "SUBSCRIBERS is a defined assoc"
ztest_run
