#!/usr/bin/env zshrs
# Word frequency counter — assoc-array accumulator.

typeset -A counts

while read -r line; do
    for w in $=line; do
        # Strip punctuation; lowercase.
        local clean=${w//[^a-zA-Z]/}
        clean=${clean:l}
        [[ -z $clean ]] && continue
        (( counts[$clean]++ ))
    done
done <<EOF
The quick brown fox jumps over the lazy dog.
The dog barks at the fox.
Fox is quick, dog is lazy.
EOF

echo "── unique words: ${#counts[@]} ──"

# Print sorted by frequency desc, then alpha asc.
for k v in "${(@kv)counts}"; do
    printf "%4d %s\n" $v $k
done | sort -k1,1 -nr -k2,2

# === ztest assertions ===
zassert_eq "${#counts[@]}" "11"   "unique word count"
zassert_eq "${counts[the]}" "4"   "the appears 4x"
zassert_eq "${counts[fox]}" "3"   "fox appears 3x"
zassert_eq "${counts[dog]}" "3"   "dog appears 3x"
zassert_eq "${counts[is]}"  "2"   "is appears 2x"
zassert_eq "${counts[quick]}" "2" "quick appears 2x"
zassert_eq "${counts[lazy]}"  "2" "lazy appears 2x"
zassert_eq "${counts[brown]}" "1" "brown appears 1x"
zassert_eq "${counts[jumps]}" "1" "jumps appears 1x"
zassert_eq "${counts[over]}"  "1" "over appears 1x"
zassert_eq "${counts[barks]}" "1" "barks appears 1x"
zassert_eq "${counts[at]}"    "1" "at appears 1x"
ztest_run
