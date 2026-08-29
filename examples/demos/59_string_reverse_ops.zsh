#!/usr/bin/env zshrs
# Advanced string operations — count chars, vowels, transforms.

count_chars() {
    typeset -A counts
    local s=$1 i ch
    for ((i = 1; i <= ${#s}; i++)); do
        ch=${s[i]}
        [[ $ch == " " ]] && continue
        (( counts[$ch]++ ))
    done
    for k v in "${(@kv)counts}"; do
        printf "%s=%d\n" "$k" "$v"
    done | sort
}

count_vowels() {
    local s=${1:l} v=0 i ch
    for ((i = 1; i <= ${#s}; i++)); do
        ch=${s[i]}
        case $ch in
            a|e|i|o|u) (( v++ )) ;;
        esac
    done
    echo $v
}

is_palindrome() {
    local s=${1:l}
    s=${s// /}
    local rev=""
    local i
    for ((i = ${#s}; i >= 1; i--)); do rev+="${s[i]}"; done
    [[ $s == $rev ]]
}

remove_duplicates() {
    local s=$1 seen="" out="" i ch
    for ((i = 1; i <= ${#s}; i++)); do
        ch=${s[i]}
        if [[ "$seen" != *"$ch"* ]]; then
            seen+="$ch"
            out+="$ch"
        fi
    done
    echo "$out"
}

echo "── char count ──"
count_chars "hello world"

echo "── vowels ──"
for s in "hello world" "zsh shell" "AEIOU" "rhythm"; do
    echo "'$s' has $(count_vowels $s) vowels"
done

echo "── palindrome ──"
for s in "racecar" "level" "hello" "A man a plan a canal Panama" "abc"; do
    if is_palindrome "$s"; then
        echo "  '$s' → yes"
    else
        echo "  '$s' → no"
    fi
done

echo "── remove duplicates ──"
echo "'mississippi' → '$(remove_duplicates mississippi)'"
echo "'aabbccddee' → '$(remove_duplicates aabbccddee)'"
echo "'hello world' → '$(remove_duplicates "hello world")'"

# === ztest assertions ===
zassert_eq "$(count_vowels 'hello world')"  3 "vowels in 'hello world'"
zassert_eq "$(count_vowels 'AEIOU')"        5 "vowels in 'AEIOU' (lowercased first)"
zassert_eq "$(count_vowels 'rhythm')"       0 "vowels in 'rhythm'"
zassert_eq "$(count_vowels 'zsh shell')"    1 "vowels in 'zsh shell'"
is_palindrome racecar && zassert_ok 1 "racecar palindrome" || zassert_ok 0 "racecar palindrome"
is_palindrome level   && zassert_ok 1 "level palindrome"   || zassert_ok 0 "level palindrome"
is_palindrome hello   && zassert_ok 0 "hello not palindrome" || zassert_ok 1 "hello not palindrome"
is_palindrome "A man a plan a canal Panama" && zassert_ok 1 "Panama phrase palindrome" || zassert_ok 0 "Panama phrase palindrome"
zassert_eq "$(remove_duplicates mississippi)" "misp"  "remove_duplicates keeps first occurrence"
zassert_eq "$(remove_duplicates aabbccddee)"  "abcde" "remove_duplicates on runs"
zassert_eq "$(remove_duplicates 'hello world')" "helo wrd" "remove_duplicates keeps the space"
ztest_run
