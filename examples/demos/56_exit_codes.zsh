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

# === ztest assertions ===
true;  zassert_eq $? 0 "true exits 0"
false; zassert_eq $? 1 "false exits 1"
return_42; zassert_eq $? 42 "return 42"
check 50;  zassert_eq $? 0 "check 50 → 0"
check 0;   zassert_eq $? 1 "check 0 → 1"
check 200; zassert_eq $? 2 "check 200 → 2"
( exit 5 ); zassert_eq $? 5 "subshell exit 5"
echo needle | grep -q needle  && zassert_ok 1 "grep found"  || zassert_ok 0 "grep found"
echo needle | grep -q missing && zassert_ok 0 "grep missing"|| zassert_ok 1 "grep missing"
setopt pipefail; false | true; zassert_eq $? 1 "pipefail: 1"
unsetopt pipefail; false | true; zassert_eq $? 0 "no pipefail: 0"
ztest_run
