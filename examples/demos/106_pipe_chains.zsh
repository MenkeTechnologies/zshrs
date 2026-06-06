#!/usr/bin/env zshrs
# Complex pipe chains — multi-stage filtering, branching, &&/||.
# Ported from Src/exec.c execpipe + Src/exec.c execlist.

echo "── 3-stage pipeline ──"
printf "banana\napple\ncherry\napple\nbanana\n" | sort -u | wc -l

echo "── filter | transform | aggregate ──"
printf "3\n1\n4\n1\n5\n9\n2\n6\n" \
    | sort -n \
    | tr '\n' ' ' \
    | sed 's/ *$/\n/'

echo "── command with conditional success ──"
echo "hello" | grep -q hello && echo "found" || echo "missing"
echo "hello" | grep -q xyz && echo "found" || echo "missing"

echo "── tee branch ──"
tmp=/tmp/zshrs_pipe_$$
echo "tee branch content" | tee "$tmp" > /dev/null
echo "captured: $(cat $tmp)"
rm -f "$tmp"

echo "── lazy pipe (only consume head) ──"
seq() { local i; for ((i=$1; i<=$2; i++)); do echo $i; done; }
seq 1 10000 | head -3

echo "── stream into a tmp file then aggregate ──"
tmp_log=/tmp/zshrs_agg_$$
seq 1 5 | tee "$tmp_log" | wc -l
echo "captured to file:"
cat "$tmp_log"
rm -f "$tmp_log"

echo "── && / || chains ──"
true && echo "T1" && true && echo "T2"
false || echo "F1" || echo "F2 unreached"
true && echo "A" && false || echo "B fired"
false || true && echo "C fired"

echo "── pipe through string builder ──"
upper=$(echo "hello world" | tr 'a-z' 'A-Z')
echo "result: $upper"

echo "── exit code through pipe (rightmost wins by default) ──"
( exit 7 ) | ( exit 11 )
echo "rightmost: $?"

set -o pipefail
( exit 7 ) | true
echo "leftmost via pipefail: $?"
set +o pipefail

# === ztest assertions ===
# 3-stage pipeline: sort -u | wc -l ⇒ 3 unique items.
count=$(printf "banana\napple\ncherry\napple\nbanana\n" | sort -u | wc -l | tr -d ' ')
zassert_eq "$count" "3"  "sort -u | wc -l"
# Conditional success chain
zassert_eq "$(echo hello | grep -q hello && echo found || echo missing)"  "found"   "&& fires on success"
zassert_eq "$(echo hello | grep -q xyz   && echo found || echo missing)"  "missing" "|| fires on failure"
# String transform via pipe
zassert_eq "$(echo 'hello world' | tr 'a-z' 'A-Z')"  "HELLO WORLD"  "tr upcase pipe"
# Sort/uniq numeric
zassert_eq "$(printf '3\n1\n2\n' | sort -n | tr '\n' ' ')"  "1 2 3 "   "numeric sort"
# rightmost exit code wins without pipefail
( exit 7 ) | ( exit 11 )
zassert_eq "$?"  "11"  "rightmost wins"
# pipefail propagates leftmost nonzero
set -o pipefail
( exit 5 ) | true
zassert_eq "$?"  "5"  "pipefail propagates left"
set +o pipefail
ztest_run
