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
