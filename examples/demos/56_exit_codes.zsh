#!/usr/bin/env zshrs
# Exit codes and $? — propagate, capture, chain via && / ||.

echo "── true / false ──"
true; echo "true → $?"
false; echo "false → $?"

echo "── explicit return ──"
return_42() { return 42; }
return_42; echo "return 42 → $?"

echo "── && / || chains ──"
true && echo "true-branch"
false || echo "false-branch"
true && true && echo "both"
false || true && echo "fallback-then-true"

echo "── grep exit ──"
echo "needle" | grep -q needle && echo "found"
echo "needle" | grep -q missing || echo "not found"

echo "── pipefail ──"
setopt pipefail
false | true; echo "with pipefail: $?"
unsetopt pipefail
false | true; echo "no pipefail: $?"

echo "── compound logical ──"
check() {
    local n=$1
    if (( n > 100 )); then return 2
    elif (( n > 0 )); then return 0
    else return 1
    fi
}
check 50; echo "check 50 → $?"
check 0; echo "check 0 → $?"
check 200; echo "check 200 → $?"

echo "── exit with code ──"
( exit 5 ); echo "subshell exit → $?"
