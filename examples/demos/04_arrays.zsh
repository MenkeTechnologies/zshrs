#!/usr/bin/env zshrs
# Arrays — declaration, indexing, slicing, append, length.
fruits=(apple banana cherry date elderberry)

echo "── basics ──"
echo "count: ${#fruits[@]}"
echo "first: ${fruits[1]}"
echo "second: ${fruits[2]}"
echo "last: ${fruits[-1]}"

echo "── slicing ──"
echo "1..3: ${fruits[1,3]}"
echo "2..end: ${fruits[2,-1]}"
echo "all: ${fruits[@]}"

echo "── iteration ──"
for x in "${fruits[@]}"; do
    echo "  - $x"
done

echo "── append ──"
fruits+=(fig)
fruits+=(grape)
echo "new count: ${#fruits[@]}"
echo "new last: ${fruits[-1]}"

echo "── reassign element ──"
fruits[1]="APPLE"
echo "${fruits[@]}"
