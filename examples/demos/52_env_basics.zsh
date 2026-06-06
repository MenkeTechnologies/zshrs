#!/usr/bin/env zshrs
# Environment variable basics — export, unset, defaults, parameter store.

echo "── export ──"
export DEMO_VAR=hello
echo "DEMO_VAR=$DEMO_VAR"
echo "subshell sees DEMO_VAR=$(echo $DEMO_VAR)"

echo "── unset / default ──"
unset DEMO_VAR
echo "after unset: DEMO_VAR=${DEMO_VAR:-fallback}"

echo "── conditional export ──"
: ${DEMO_NEW:=initialized}
echo "DEMO_NEW=$DEMO_NEW"

echo "── env vs local ──"
LOCAL_ONLY=local_value
export EXPORTED_VAR=exported_value
zshrs_sub_check() {
    # Local fn — both vars visible within the SAME shell.
    echo "  in fn: LOCAL_ONLY=$LOCAL_ONLY EXPORTED_VAR=$EXPORTED_VAR"
}
zshrs_sub_check

echo "── parameter expansion w/ env ──"
PATH_SEGMENTS=( "${(@s/:/)PATH}" )
echo "PATH has ${#PATH_SEGMENTS[@]} segment(s) (showing first 3)"
for i in 1 2 3; do
    [[ -n ${PATH_SEGMENTS[$i]} ]] && echo "  [$i] ${PATH_SEGMENTS[$i]}"
done

echo "── cleanup ──"
unset DEMO_NEW EXPORTED_VAR LOCAL_ONLY PATH_SEGMENTS
echo "after cleanup: ${DEMO_NEW:-cleared} ${EXPORTED_VAR:-cleared}"

# === ztest assertions ===
export RETEST_VAR=on
zassert_eq "$RETEST_VAR"     "on"        "export visible in same shell"
zassert_eq "$(echo $RETEST_VAR)" "on"    "export visible in subshell"
unset RETEST_VAR
zassert_eq "${RETEST_VAR:-cleared}" "cleared" "unset triggers :- fallback"
RETEST_DEF=
: ${RETEST_DEF:=initialized}
zassert_eq "$RETEST_DEF"     "initialized" ":= sets when empty"
: ${RETEST_DEF:=second}
zassert_eq "$RETEST_DEF"     "initialized" ":= is no-op when set"
zassert_eq "${DEMO_NEW:-cleared}"      "cleared" "DEMO_NEW gone after cleanup"
zassert_eq "${EXPORTED_VAR:-cleared}"  "cleared" "EXPORTED_VAR gone after cleanup"
unset RETEST_DEF
ztest_run
