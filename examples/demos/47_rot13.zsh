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

# === ztest assertions ===
zassert_eq "$(rot13 'Hello, World!')"  "Uryyb, Jbeyq!"                 "rot13 hello world"
zassert_eq "$(rot13 'zsh is great')"   "mfu vf terng"                   "rot13 zsh"
zassert_eq "$(rot13 'abcdefghijklmnopqrstuvwxyz')" "nopqrstuvwxyzabcdefghijklm" "rot13 alphabet"
zassert_eq "$(rot13 "$(rot13 'roundtrip')")" "roundtrip"                "rot13 self-inverse"
zassert_eq "$plain" "$back"                                              "round-trip preserved"
zassert_eq "$enc"   "Gur dhvpx oebja sbk whzcf bire gur ynml qbt"        "encoded fox sentence"
ztest_run
