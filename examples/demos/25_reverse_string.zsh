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

# === ztest assertions ===
zassert_eq "$(reverse hello)"   "olleh"   "reverse hello"
zassert_eq "$(reverse zshrs)"   "srhsz"   "reverse zshrs"
zassert_eq "$(reverse racecar)" "racecar" "reverse racecar (palindrome)"
zassert_eq "$(reverse ABCDEFG)" "GFEDCBA" "reverse ABCDEFG"
zassert_eq "$(reverse a)"       "a"       "reverse single char"
zassert_eq "$(reverse '')"      ""        "reverse empty string"
if is_palindrome racecar; then zassert_ok 1 "racecar palindrome"; else zassert_ok 0 "racecar palindrome"; fi
if is_palindrome level;   then zassert_ok 1 "level palindrome";   else zassert_ok 0 "level palindrome";   fi
if is_palindrome madam;   then zassert_ok 1 "madam palindrome";   else zassert_ok 0 "madam palindrome";   fi
if is_palindrome hello;   then zassert_ok 0 "hello not palindrome"; else zassert_ok 1 "hello not palindrome"; fi
if is_palindrome abc;     then zassert_ok 0 "abc not palindrome"; else zassert_ok 1 "abc not palindrome"; fi
ztest_run
