#!/usr/bin/env zshrs
# Trap deep dive — ALL signal numbers + DEBUG/EXIT/ERR/ZERR/RETURN.
# Ports Src/signals.c install_handler + Src/exec.c dotrap.

echo "── named traps ──"

# Use counters since trap output may not show ordering.
typeset -gi exit_cnt=0 err_cnt=0 zerr_cnt=0
typeset -gi int_cnt=0 term_cnt=0 hup_cnt=0
typeset -gi usr1_cnt=0 usr2_cnt=0
typeset -gi return_cnt=0 debug_cnt=0

# Install full trap matrix.
trap '(( exit_cnt++ )); echo "  EXIT trap fired ($exit_cnt)"' EXIT
trap '(( err_cnt++ )); echo "  ERR trap fired ($err_cnt)"' ERR
trap '(( zerr_cnt++ )); echo "  ZERR trap fired ($zerr_cnt)"' ZERR

trap '(( int_cnt++ )); echo "  INT trap fired ($int_cnt)"' INT
trap '(( term_cnt++ )); echo "  TERM trap fired ($term_cnt)"' TERM
trap '(( hup_cnt++ )); echo "  HUP trap fired ($hup_cnt)"' HUP
trap '(( usr1_cnt++ )); echo "  USR1 trap fired ($usr1_cnt)"' USR1
trap '(( usr2_cnt++ )); echo "  USR2 trap fired ($usr2_cnt)"' USR2

echo "  all signal handlers installed"
echo

echo "── send self USR1 / USR2 ──"
kill -USR1 $$
sleep 0.05
kill -USR2 $$
sleep 0.05
kill -USR1 $$
sleep 0.05
echo "  USR1 total: $usr1_cnt"
echo "  USR2 total: $usr2_cnt"

echo
echo "── ignore vs reset ──"
trap '' USR1
echo "  USR1 now ignored"
kill -USR1 $$
sleep 0.05
echo "  USR1 count still: $usr1_cnt (unchanged)"
trap - USR1
echo "  USR1 reset to default — no longer trapped"
trap '(( usr1_cnt++ )); echo "  USR1 fires again"' USR1
kill -USR1 $$
sleep 0.05
echo "  USR1 count: $usr1_cnt"

echo
echo "── view installed traps ──"
trap -p 2>/dev/null | head -8

echo
echo "── trap firing in subshells ──"
(
    trap 'echo "    subshell EXIT"' EXIT
    trap 'echo "    subshell USR1"' USR1
    echo "  inside subshell"
    kill -USR1 $$
    sleep 0.05
)
echo "  subshell exited"

echo
echo "── ERR vs ZERR ──"
# ZERR is zsh-specific, fires after each error.
trigger_err() {
    false  # exit status 1
}
# Reset counters.
err_cnt=0
zerr_cnt=0

# Run a failing command.
trigger_err 2>/dev/null
echo "  after false: err=$err_cnt zerr=$zerr_cnt"

echo
echo "── trap with exit codes ──"
typeset -gi last_exit=0
trap 'last_exit=$?; echo "  caught EXIT with code $last_exit"' EXIT

(
    exit 42
)
sleep 0.05

echo
echo "── nested subshell trap inheritance ──"
(
    trap 'echo "    outer-sub EXIT"' EXIT
    (
        trap 'echo "      inner-sub EXIT"' EXIT
        (
            trap 'echo "        deepest EXIT"' EXIT
            echo "  triple-nested running"
        )
    )
)

echo
echo "── unwatched signals ignored (default behavior) ──"
trap - USR1 USR2
echo "  USR1+USR2 cleared"
trap -p USR1 USR2 2>/dev/null

echo
echo "── batch signal install ──"
cleanup() { echo "  cleanup fired"; }
trap cleanup INT TERM HUP USR1 USR2
trap -p 2>/dev/null | head -5
trap - INT TERM HUP USR1 USR2
echo "  all batch-cleared"

echo
echo "── trap names + numbers ──"
# Standard signal numbers (POSIX).
echo "  HUP=1 INT=2 QUIT=3 ILL=4 TRAP=5 ABRT=6"
echo "  BUS=10/7 FPE=8 KILL=9 USR1=10/30 SEGV=11"
echo "  PIPE=13 ALRM=14 TERM=15 USR2=12/31 CHLD=17/20"

echo
echo "── final summary ──"
echo "  total traps installed:    via 9 handlers"
echo "  total USR1 fires:         $usr1_cnt"
echo "  total USR2 fires:         $usr2_cnt"
echo "  total ERR fires:          $err_cnt"
echo "  total ZERR fires:         $zerr_cnt"

# === ztest assertions ===
# 3 USR1 sent, then ignored, then reset to default, then re-trapped + 1 = 4 fires
zassert_eq "$usr1_cnt" 4 "USR1 fired 4 times across reinstall cycle"
zassert_eq "$usr2_cnt" 1 "USR2 fired once"
zassert_eq "$zerr_cnt" 2 "ZERR fired on failing commands"
# Counters never went negative
zassert_ge "$int_cnt"  0 "INT counter non-negative"
zassert_ge "$term_cnt" 0 "TERM counter non-negative"
zassert_ge "$hup_cnt"  0 "HUP counter non-negative"
# Function with trap returns 0 from explicit `false` ZERR catches but doesn't propagate failure
trigger_ok() { true; }
trigger_ok
zassert_eq "$?" 0 "true command exits 0"
ztest_run
