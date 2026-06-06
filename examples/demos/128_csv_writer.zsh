#!/usr/bin/env zshrs
# CSV writer/reader in pure zsh — proper quoting + escaping.

csv_escape_field() {
    local field=$1
    # Quote if it contains comma, newline, or double-quote.
    if [[ $field == *,* || $field == *$'\n'* || $field == *\"* ]]; then
        # Double-up internal quotes.
        local escaped=${field//\"/\"\"}
        echo "\"${escaped}\""
    else
        echo "$field"
    fi
}

csv_write_row() {
    local first=1 field
    for field in "$@"; do
        if (( first )); then
            first=0
        else
            printf ","
        fi
        printf "%s" "$(csv_escape_field "$field")"
    done
    printf "\n"
}

echo "── basic ──"
csv_write_row Name Age Role
csv_write_row Alice 30 Admin
csv_write_row Bob 25 User

echo "── with embedded special chars ──"
csv_write_row "Name with, comma" "Value" "plain"
csv_write_row "She said \"hello\"" 42 "OK"
csv_write_row "multi
line" "another" "field"

echo "── full table ──"
csv_write_row ID Name Email Role
csv_write_row 1 "Alice, A." "alice@example.com" admin
csv_write_row 2 "Bob \"the builder\"" "bob@example.com" user
csv_write_row 3 "Carol" "carol@example.com" "guest, temp"

echo "── reading a CSV row (simple, no embedded quotes/commas) ──"
csv_read_simple() {
    local line=$1
    local IFS=,
    local -a fields=( ${(s/,/)line} )
    for f in "${fields[@]}"; do
        echo "  field: '$f'"
    done
}
echo "simple row:"
csv_read_simple "alice,30,admin"

# === ztest assertions ===
zassert_eq "$(csv_escape_field plain)"                 "plain"                          "no quoting needed"
zassert_eq "$(csv_escape_field 'has,comma')"           '"has,comma"'                    "comma forces quoting"
zassert_eq "$(csv_escape_field 'she said "hi"')"       '"she said ""hi"""'              "embedded quote doubled"
zassert_eq "$(csv_write_row a b c)"                    "a,b,c"                          "row basic"
zassert_eq "$(csv_write_row Alice 30 Admin)"           "Alice,30,Admin"                 "row people"
zassert_eq "$(csv_write_row 'x, y' z)"                 '"x, y",z'                       "row with commas"
zassert_contains "$(csv_write_row 1 'Alice, A.' alice@example.com admin)" '"Alice, A."'  "field with comma quoted"
ztest_run
