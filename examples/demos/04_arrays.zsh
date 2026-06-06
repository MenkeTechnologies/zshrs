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

# === ztest assertions ===
# count after two appends is 7 (5 original + fig + grape).
zassert_eq "${#fruits[@]}"  7              "post-append count"
zassert_eq "${fruits[-1]}"  "grape"        "post-append last"
zassert_eq "${fruits[1]}"   "APPLE"        "reassigned element"
zassert_eq "${fruits[2]}"   "banana"       "untouched element"
# rebuild a fresh array so we can assert pre-append shape too.
fresh=(apple banana cherry date elderberry)
zassert_eq "${#fresh[@]}"   5              "fresh count"
zassert_eq "${fresh[1]}"    "apple"        "fresh first"
zassert_eq "${fresh[3]}"    "cherry"       "fresh third"
zassert_eq "${fresh[1,3]}"  "apple banana cherry" "slice 1..3"
zassert_eq "${fresh[2,-1]}" "banana cherry date elderberry" "slice 2..end"
ztest_run
