#!/usr/bin/env zshrs
# Retry-with-backoff wrapper.

retry() {
    local max_attempts=$1
    shift
    local base_delay_ms=${RETRY_BASE_MS:-50}
    local attempt=1
    while (( attempt <= max_attempts )); do
        if "$@"; then
            echo "  succeeded on attempt $attempt"
            return 0
        fi
        local delay_ms=$(( base_delay_ms * (1 << (attempt - 1)) ))
        echo "  attempt $attempt failed; sleeping ${delay_ms}ms"
        # Convert to fractional seconds.
        local sec=$(( delay_ms / 1000 ))
        local rem_ms=$(( delay_ms % 1000 ))
        sleep "$sec.$(printf '%03d' $rem_ms)"
        (( attempt++ ))
    done
    echo "  all $max_attempts attempts failed"
    return 1
}

# Mock command that fails N times then succeeds.
typeset -i FAIL_COUNT=3
flaky_cmd() {
    if (( FAIL_COUNT > 0 )); then
        (( FAIL_COUNT-- ))
        return 1
    fi
    return 0
}

echo "── retry success after 3 failures ──"
FAIL_COUNT=3
retry 5 flaky_cmd

echo "── retry exhausted ──"
FAIL_COUNT=10
retry 3 flaky_cmd

echo "── immediate success ──"
FAIL_COUNT=0
retry 5 flaky_cmd

echo "── with custom base delay ──"
RETRY_BASE_MS=20
FAIL_COUNT=2
retry 4 flaky_cmd

echo "── retry-until-condition pattern ──"
target_value=42
typeset -i counter=0
condition() {
    (( counter++ ))
    (( counter == target_value / 10 ))
}
retry 10 condition
echo "counter reached: $counter"

echo "── exponential delay schedule (no sleep, just print) ──"
schedule() {
    local n=$1 base=${2:-50}
    for ((i=1; i<=n; i++)); do
        echo "  attempt $i: wait $(( base * (1 << (i - 1)) ))ms"
    done
}
schedule 5
schedule 5 100

# === ztest assertions ===
# Schedule should produce correct exponential backoff values (no sleep, fast)
zassert_contains "$(schedule 3)" "wait 50ms"   "first delay 50ms"
zassert_contains "$(schedule 3)" "wait 100ms"  "second delay 100ms"
zassert_contains "$(schedule 3)" "wait 200ms"  "third delay 200ms"
zassert_contains "$(schedule 3 100)" "wait 100ms" "base=100 first"
zassert_contains "$(schedule 3 100)" "wait 400ms" "base=100 third"
# flaky_cmd state checks
FAIL_COUNT=2
flaky_cmd && r=1 || r=0; zassert_eq "$r" 0 "first call fails when FAIL_COUNT=2"
flaky_cmd && r=1 || r=0; zassert_eq "$r" 0 "second call fails"
flaky_cmd && r=1 || r=0; zassert_eq "$r" 1 "third call succeeds"
zassert_eq "$FAIL_COUNT" 0 "FAIL_COUNT exhausted to 0"
ztest_run
