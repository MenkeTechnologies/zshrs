#!/usr/bin/env zshrs
# Read file → array (mapfile-like pattern).

tmpfile="/tmp/zshrs_mapfile_$$"
cat > "$tmpfile" <<EOF
line one
line two
line three
line four
line five
EOF

trap "rm -f $tmpfile" EXIT

echo "── read file into array ──"
lines=( "${(@f)$(<$tmpfile)}" )
echo "got ${#lines[@]} lines"
for ((i = 1; i <= ${#lines[@]}; i++)); do
    printf "[%2d] %s\n" $i "${lines[i]}"
done

echo "── reverse order ──"
for ((i = ${#lines[@]}; i >= 1; i--)); do
    echo "${lines[i]}"
done

echo "── filter (skip lines containing 'two') ──"
for l in "${lines[@]}"; do
    [[ $l != *two* ]] && echo "$l"
done

echo "── word count via array ──"
total=0
for l in "${lines[@]}"; do
    words=( $=l )
    (( total += ${#words[@]} ))
done
echo "total words: $total"
