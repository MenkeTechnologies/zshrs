#!/usr/bin/env zshrs
# Atbash cipher — reverse alphabet substitution; self-inverse.

atbash() {
    echo "$1" | tr 'A-Za-z' 'Z-Aa-z'
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
