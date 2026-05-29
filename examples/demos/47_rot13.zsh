#!/usr/bin/env zshrs
# ROT13 — Caesar cipher with shift 13 (self-inverse).

rot13() {
    echo "$1" | tr 'A-Za-z' 'N-ZA-Mn-za-m'
}

echo "── encode ──"
rot13 "Hello, World!"
rot13 "zsh is great"
rot13 "abcdefghijklmnopqrstuvwxyz"

echo "── round-trip ──"
plain="The quick brown fox jumps over the lazy dog"
enc=$(rot13 "$plain")
back=$(rot13 "$enc")
echo "plain: $plain"
echo "enc:   $enc"
echo "back:  $back"
[[ "$plain" == "$back" ]] && echo "round-trip OK" || echo "round-trip FAIL"
