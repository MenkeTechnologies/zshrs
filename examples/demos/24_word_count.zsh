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
