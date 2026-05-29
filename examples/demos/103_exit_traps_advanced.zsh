#!/usr/bin/env zshrs
# Exit/ERR/DEBUG/ZERR traps — accumulator, multi-handler patterns.
# Ported from Src/signals.c handletrap + Src/exec.c execsignals.

echo "── basic EXIT trap ──"
(
    trap 'echo "EXIT trap subshell"' EXIT
    echo "doing work"
)

echo "── EXIT cleans up tempdir ──"
cleanup_tmp() {
    rm -rf "$tmp"
    echo "cleaned $tmp"
}
tmp=/tmp/zshrs_trap_test_$$
mkdir -p "$tmp"
trap cleanup_tmp EXIT
echo "tmp dir: $tmp"
echo "exists: $(test -d $tmp && echo yes)"
# cleanup_tmp will fire on script exit.

echo "── ERR trap ──"
(
    trap 'echo "ERR: $?"' ERR
    false
    true
    false  # would trigger again
) 2>/dev/null

echo "── trap reset ──"
trap 'echo "first"' EXIT
trap - EXIT
echo "(no EXIT trap → no first output on exit)"

echo "── chained trap (preserve previous) ──"
(
    trap 'echo "A1"' EXIT
    # Capture existing trap to chain it.
    prev_trap=$(trap -p EXIT 2>/dev/null)
    trap 'echo "A2"; eval "$prev_trap"' EXIT
)

echo "── signal-named traps ──"
(
    trap 'echo "INT received"' INT
    # Simulate normally via a kill in subshell - skip for non-interactive
    echo "INT trap installed (skipped firing)"
)

echo "── ZERR trap (zsh-specific, on error) ──"
(
    trap 'echo "ZERR fired"' ZERR
    false
) 2>/dev/null

echo "── multi-handler via fn dispatch ──"
log_event() {
    echo "EVENT: $1 at $(date +%T 2>/dev/null || echo time)"
}
(
    trap 'log_event ENTER' EXIT
    echo "block body"
)
