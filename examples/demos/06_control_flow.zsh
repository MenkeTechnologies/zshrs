#!/usr/bin/env zshrs
# Control flow — if/elif/else, case, while, until.
n=15

echo "── if/elif/else ──"
if (( n > 100 )); then
    echo "big"
elif (( n > 10 )); then
    echo "medium"
elif (( n > 0 )); then
    echo "small"
else
    echo "zero or negative"
fi

echo "── case ──"
case $n in
    0) echo "zero" ;;
    [1-9]) echo "single digit" ;;
    1[0-9]) echo "teens" ;;
    *) echo "big" ;;
esac

echo "── while ──"
i=0
while (( i < 3 )); do
    echo "while i=$i"
    (( i++ ))
done

echo "── until ──"
i=0
until (( i >= 3 )); do
    echo "until i=$i"
    (( i++ ))
done

echo "── break/continue ──"
for i in {1..6}; do
    (( i == 4 )) && continue
    (( i == 6 )) && break
    echo "loop i=$i"
done

# === ztest assertions ===
classify() {
    local x=$1
    if   (( x > 100 )); then echo big
    elif (( x > 10 ));  then echo medium
    elif (( x > 0 ));   then echo small
    else                    echo zero-or-neg
    fi
}

zassert_eq "$(classify 200)" "big"        "if classify big"
zassert_eq "$(classify 15)"  "medium"     "if classify medium (n=15)"
zassert_eq "$(classify 5)"   "small"      "if classify small"
zassert_eq "$(classify 0)"   "zero-or-neg" "if classify zero"
case_classify() {
    case $1 in
        0)      echo zero ;;
        [1-9])  echo single ;;
        1[0-9]) echo teens ;;
        *)      echo big ;;
    esac
}
zassert_eq "$(case_classify 0)"  "zero"   "case zero"
zassert_eq "$(case_classify 7)"  "single" "case single"
zassert_eq "$(case_classify 15)" "teens"  "case teens"
zassert_eq "$(case_classify 99)" "big"    "case fallthrough"
# loop with break/continue: emits i=1,2,3,5 — 4 skipped, 6 breaks
seen=()
for i in {1..6}; do
    (( i == 4 )) && continue
    (( i == 6 )) && break
    seen+=("$i")
done
zassert_eq "${(j: :)seen}" "1 2 3 5" "break/continue trace"
ztest_run
