#!/usr/bin/env zshrs
# IFS-based field splitting.

echo "── space split ──"
sentence="one two three four"
for w in $=sentence; do
    echo "word=$w"
done

echo "── colon split ──"
path="/usr/local/bin:/usr/bin:/bin"
IFS=":" parts=( $=path )
for p in "${parts[@]}"; do
    echo "path: $p"
done

echo "── csv split ──"
csv="alice,30,admin"
IFS="," fields=( $=csv )
echo "name=${fields[1]}"
echo "age=${fields[2]}"
echo "role=${fields[3]}"

echo "── manual split via parameter flag ──"
joined="a:b:c:d"
print -l "${(s/:/)joined}"

# === ztest assertions ===
zassert_eq "$sentence" "one two three four" "sentence var"
zassert_eq "${parts[1]}" "/usr/local/bin"   "colon split parts[1]"
zassert_eq "${parts[2]}" "/usr/bin"          "colon split parts[2]"
zassert_eq "${parts[3]}" "/bin"              "colon split parts[3]"
zassert_eq "${fields[1]}" "alice"            "csv name"
zassert_eq "${fields[2]}" "30"               "csv age"
zassert_eq "${fields[3]}" "admin"            "csv role"
# Easier: array assignment from (s) flag
sp=( ${(s/:/)joined} )
zassert_eq "${#sp[@]}" 4 "(s/:/) produced 4 elements"
zassert_eq "${sp[1]}" "a" "(s) split first"
zassert_eq "${sp[4]}" "d" "(s) split last"
ztest_run
