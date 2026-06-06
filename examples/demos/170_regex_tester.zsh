#!/usr/bin/env zshrs
# Regex tester — drives [[ str =~ pat ]] over a corpus.

zmodload zsh/regex 2>/dev/null || true

test_pattern() {
    local pat=$1
    shift
    echo "── pattern: '$pat' ──"
    for input in "$@"; do
        if [[ $input =~ $pat ]]; then
            local captures=""
            if (( ${#match[@]} > 0 )); then
                for ((i=1; i<=${#match[@]}; i++)); do
                    captures+=" g$i='${match[i]}'"
                done
            fi
            printf "  YES: %-30s match='%s'%s\n" "'$input'" "$MATCH" "$captures"
        else
            printf "  no:  '%s'\n" "$input"
        fi
    done
}

test_pattern '^[0-9]+$' \
    "12345" "abc" "12a" "" "0" "999999"

test_pattern '^[a-z]+@[a-z]+\.[a-z]+$' \
    "alice@example.com" "BAD@MAIL" "no-at-sign" "user@host.org"

test_pattern '^([A-Z][a-z]+) ([A-Z][a-z]+)$' \
    "John Smith" "alice jones" "Bob A" "Carol Smith"

test_pattern '([0-9]{4})-([0-9]{2})-([0-9]{2})' \
    "2026-05-29 is today" "not a date" "wrong-format" "1999-01-01 was Y2K"

test_pattern 'https?://([^/]+)' \
    "https://example.com/path" "http://other.org/x" "ftp://nope.com" "plain text"

test_pattern '^#([0-9a-fA-F]{6})$' \
    "#ff0000" "#0F0F0F" "no-hash-AAA" "#tooshort" "#GGGGGG"

echo "── pattern from file (one per line, conceptual) ──"
patterns=("[a-z]+ " "[0-9]+" "^[A-Z]" "world$")
sentence="Hello World 42 is a magic number"
for pat in "${patterns[@]}"; do
    if [[ $sentence =~ $pat ]]; then
        echo "matches '$pat': MATCH=$MATCH"
    fi
done

# === ztest assertions ===
# Numeric pattern.
[[ "12345" =~ '^[0-9]+$' ]] && r1=ok || r1=no
zassert_eq "$r1" "ok"          "all-digits matches ^[0-9]+\$"
[[ "12a"   =~ '^[0-9]+$' ]] && r2=ok || r2=no
zassert_eq "$r2" "no"          "12a does not match ^[0-9]+\$"
# Email pattern.
[[ "alice@example.com" =~ '^[a-z]+@[a-z]+\.[a-z]+$' ]] && r3=ok || r3=no
zassert_eq "$r3" "ok"          "email matches"
# Name pattern with capture groups.
[[ "John Smith" =~ '^([A-Z][a-z]+) ([A-Z][a-z]+)$' ]]
zassert_eq "${match[1]}" "John"  "first capture = John"
zassert_eq "${match[2]}" "Smith" "second capture = Smith"
# Hex color match.
[[ "#ff0000" =~ '^#([0-9a-fA-F]{6})$' ]] && r4=ok || r4=no
zassert_eq "$r4" "ok"           "#ff0000 matches hex pattern"
[[ "#tooshort" =~ '^#([0-9a-fA-F]{6})$' ]] && r5=ok || r5=no
zassert_eq "$r5" "no"           "#tooshort rejected"
# MATCH variable populated.
[[ "Hello World 42" =~ '[0-9]+' ]]
zassert_eq "$MATCH" "42"        "MATCH variable captures number"
ztest_run
