#!/usr/bin/env zshrs
# Atbash cipher — reverse alphabet substitution; self-inverse.

atbash() {
    echo "$1" | tr 'A-Za-z' 'ZYXWVUTSRQPONMLKJIHGFEDCBAzyxwvutsrqponmlkjihgfedcba'
}

echo "── encode ──"
atbash "Hello, World!"
atbash "abcdefghijklmnopqrstuvwxyz"
atbash "Zsh Run Right"

echo "── round-trip ──"
plain="The quick brown fox"
enc=$(atbash "$plain")
back=$(atbash "$enc")
echo "plain: $plain"
echo "enc:   $enc"
echo "back:  $back"
[[ "$plain" == "$back" ]] && echo "round-trip OK" || echo "round-trip FAIL"

# === ztest assertions ===
# The reversed alphabets are spelled out: a descending range such as 'Z-A' is
# not portable (GNU tr rejects it, BSD tr reads it differently).
zassert_eq "$(atbash 'Hello, World!')"  "Svool, Dliow!"               "atbash mixed case"
zassert_eq "$(atbash 'abcdefghijklmnopqrstuvwxyz')" "zyxwvutsrqponmlkjihgfedcba" "atbash lowercase alphabet"
zassert_eq "$enc"  "Gsv jfrxp yildm ulc" "atbash encoded fox"
zassert_eq "$plain" "$back" "atbash is self-inverse"
ztest_run
