#!/usr/bin/env zshrs
# zparseopts — argv parsing builtin.
# Ported from zsh's Src/Modules/zutil.c bin_zparseopts.

parse_demo() {
    local -a verbose=() debug=() output=()
    # -D = remove parsed opts from $@
    # -E = stop at first non-option (don't shuffle)
    # name=array → append match to array
    # name+=array → accumulate
    # name:=array → option takes an arg
    zparseopts -D -E -- \
        v=verbose -verbose=verbose \
        d=debug -debug=debug \
        o:=output -output:=output \
        h=help -help=help
    echo "verbose=$verbose"
    echo "debug=$debug"
    echo "output=$output"
    echo "help=$help"
    echo "remaining args: $*"
    echo
}

echo "── basic short opts ──"
parse_demo -v -d arg1 arg2

echo "── long opts ──"
parse_demo --verbose --debug arg1 arg2

echo "── option with required arg ──"
parse_demo -o /tmp/out.log file1 file2

echo "── long opt with required arg ──"
parse_demo --output=/tmp/o.txt --verbose run.sh

echo "── help flag ──"
parse_demo --help

echo "── stop at first non-option (-E) ──"
parse_demo -v arg1 -d arg2    # -d after arg1 stays in args

echo "── repeated accumulator ──"
parse_demo_accum() {
    local -a verbose=()
    zparseopts -D -E -- v+=verbose
    echo "verbose times: ${#verbose[@]} (entries: $verbose)"
}
parse_demo_accum -v -v -v
parse_demo_accum -v
parse_demo_accum

# === ztest assertions ===
# zparseopts is stateful — easier to test via a dedicated harness fn.
harness() {
    local -a verbose=() debug=() output=()
    zparseopts -D -E -- v=verbose -verbose=verbose \
        d=debug -debug=debug \
        o:=output -output:=output
    printf "v=%s|d=%s|o=%s|rest=%s\n" \
        "${verbose[*]}" "${debug[*]}" "${output[*]}" "$*"
}
# `v` and `d` are flag specs (no `:`), so they never consume the next word —
# -E stops at the first non-option and everything from there stays in $@.
zassert_eq "$(harness -v arg1)"           "v=-v|d=|o=|rest=arg1"               "flag -v leaves arg1 in \$@"
zassert_eq "$(harness --verbose foo)"     "v=--verbose|d=|o=|rest=foo"         "flag --verbose leaves foo in \$@"
zassert_eq "$(harness -v -d a b)"         "v=-v|d=-d|o=|rest=a b"              "two flags, two positionals left"
zassert_eq "$(harness -o /tmp/x file1)"   "v=|d=|o=-o /tmp/x|rest=file1"       "o: consumes its argument"
accum() {
    local -a verbose=()
    zparseopts -D -E -- v+=verbose
    echo "${#verbose[@]}"
}
zassert_eq "$(accum -v -v -v)"   3   "accumulator -v -v -v"
zassert_eq "$(accum -v)"         1   "accumulator -v"
zassert_eq "$(accum)"            0   "accumulator empty"
zassert_eq "$(accum -v -v -v -v -v)" 5 "accumulator x5"
ztest_run

