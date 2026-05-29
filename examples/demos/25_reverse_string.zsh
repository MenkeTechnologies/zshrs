#!/usr/bin/env zshrs
# Reverse a string — character by character.
reverse() {
    local s=$1
    local rev=""
    local i
    for ((i = ${#s}; i > 0; i--)); do
        rev+="${s[i]}"
    done
    echo "$rev"
}

for s in "hello" "zshrs" "racecar" "ABCDEFG" "a"; do
    echo "$s -> $(reverse $s)"
done

# Palindrome check.
is_palindrome() {
    local s=${1:l}
    [[ "$s" == "$(reverse $s)" ]]
}

for word in racecar level hello world madam stats abc; do
    if is_palindrome $word; then
        echo "$word: palindrome"
    else
        echo "$word: not palindrome"
    fi
done
