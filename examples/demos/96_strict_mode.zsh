#!/usr/bin/env zshrs
# Strict mode flags — set -e, set -u, set -o pipefail, NO_UNSET.
# Ported from Src/options.c bin_set + runtime guards across exec/subst.

echo "── set -u (no_unset) — fail on unset var ──"
(
    set -u
    x=defined
    echo "x=$x"
    # echo "$y"  # would error: y unset
)
echo "── after subshell ──"

echo "── set -e (errexit) — fail on first error ──"
(
    set -e
    echo "before"
    true     # OK
    echo "between"
    # false  # would terminate the subshell
    echo "after"
)

echo "── set -o pipefail — propagate first failure in pipeline ──"
set +o pipefail
false | true; echo "no pipefail: $?"
set -o pipefail
false | true; echo "pipefail: $?"
set +o pipefail

echo "── trap ERR (run on error) ──"
trap 'echo "ERR trap fired"' ERR
( false ) 2>/dev/null
trap - ERR

echo "── set -n (no-exec parse-check) ──"
(
    set -n
    # Body is parsed only, not executed.
    echo "this never runs"
    invalid_syntax_xxx
)
echo "after parse-only subshell: $?"

echo "── \${var:?msg} as error guard ──"
unset critical
err=$( ( echo "${critical:?missing critical config}" ) 2>&1 )
echo "err captured: $err"

echo "── strict mode pattern function ──"
strict() {
    set -euo pipefail
    echo "all three flags on"
    echo "no_unset: ${set_test:-fallback}"  # :- avoids unset
    return 0
}
strict
echo "outer: \$? = $?"
