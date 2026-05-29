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
