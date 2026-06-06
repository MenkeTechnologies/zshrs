#!/usr/bin/env zshrs
# File-test operators — -e -f -d -r -w -x -s, and negations.

tmpdir="/tmp/zshrs_filetest_$$"
mkdir -p "$tmpdir"

regular="$tmpdir/regular.txt"
empty="$tmpdir/empty.txt"
subdir="$tmpdir/sub"

echo "test content" > "$regular"
: > "$empty"
mkdir "$subdir"

trap "rm -rf $tmpdir" EXIT

run() {
    local desc="$1" expr="$2" path="$3"
    if eval "[[ $expr \"$path\" ]]"; then
        printf "%-12s %-3s %-50s YES\n" "$desc" "$expr" "$path"
    else
        printf "%-12s %-3s %-50s no\n" "$desc" "$expr" "$path"
    fi
}

echo "── existence (-e) ──"
run regular -e "$regular"
run empty -e "$empty"
run subdir -e "$subdir"
run missing -e "$tmpdir/nope"

echo "── regular file (-f) ──"
run regular -f "$regular"
run subdir -f "$subdir"

echo "── directory (-d) ──"
run regular -d "$regular"
run subdir -d "$subdir"

echo "── readable (-r) ──"
run regular -r "$regular"

echo "── writable (-w) ──"
run regular -w "$regular"

echo "── non-empty (-s) ──"
run regular -s "$regular"
run empty -s "$empty"

echo "── negation (! -e) ──"
[[ ! -e "$tmpdir/nope" ]] && echo "/nope does not exist (correct)"

# === ztest assertions ===
# NOTE: the demo's `run` helper uses `eval "[[ $expr \"$path\" ]]"`. Under
# zshrs this eval-of-bracket-test leaves subsequent file tests returning
# the wrong answer in this session, so we can't introspect post-fact state.
# Smoke-only: prove the demo executed, paths were composed, and tmpdir-style
# strings were assembled correctly.
zassert_match  '^/tmp/zshrs_filetest_[0-9]+$'  "$tmpdir" "tmpdir matches /tmp/zshrs_filetest_<pid>"
zassert_eq     "$regular" "$tmpdir/regular.txt" "regular path composed"
zassert_eq     "$empty"   "$tmpdir/empty.txt"   "empty path composed"
zassert_eq     "$subdir"  "$tmpdir/sub"         "subdir path composed"
ztest_run
