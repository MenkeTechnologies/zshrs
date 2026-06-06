#!/usr/bin/env zshrs
# Command substitution — $(cmd) capture, nesting, arithmetic.

echo "── basic ──"
greeting=$(echo "hello from subshell")
echo "got: $greeting"

echo "── arithmetic inside ──"
words=$(echo "alpha beta gamma" | wc -w)
echo "word count: $words"

echo "── nested substitution ──"
upper=$(echo $(echo "lowercase") | tr a-z A-Z)
echo "upper: $upper"

echo "── capture into array ──"
files=( $(echo "one two three four") )
echo "n=${#files[@]}"
for f in "${files[@]}"; do
    echo "  - $f"
done

echo "── env capture ──"
echo "USER (from env): $(printenv USER 2>/dev/null || echo unknown)"

echo "── arithmetic via cmd subst ──"
n=$(echo "42")
echo "doubled: $(( n * 2 ))"

# === ztest assertions ===
zassert_eq "$(echo hi)"                 "hi"                "basic $(echo)"
zassert_eq "$(echo "with spaces")"      "with spaces"       "preserves spaces"
# Nested.
zassert_eq "$(echo $(echo nested))"     "nested"            "nested cmd subst"
zassert_eq "$(echo $(echo lower) | tr a-z A-Z)" "LOWER"     "pipe inside cmd subst"
# Array capture from cmd subst.
arr=( $(echo one two three) )
zassert_eq "${#arr[@]}"  3                                  "3-element capture"
zassert_eq "${arr[1]}"   "one"                              "first element"
zassert_eq "${arr[3]}"   "three"                            "third element"
# Arithmetic on captured value.
n_local=$(echo 21)
zassert_eq "$(( n_local * 2 ))" 42                          "arith on captured value"
# wc -w via cmd subst yields a positive count.
wc_out=$(echo "alpha beta gamma" | wc -w | tr -d ' ')
zassert_eq "$wc_out" 3                                      "wc -w via cmd subst"
ztest_run
