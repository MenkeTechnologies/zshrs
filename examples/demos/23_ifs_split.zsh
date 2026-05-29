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
