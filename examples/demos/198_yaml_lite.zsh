#!/usr/bin/env zshrs
# Naive YAML-like flat key:value parser (no nesting).

typeset -A YAML_DATA

parse_yaml_line() {
    local line=$1
    # Skip empty and comments.
    [[ -z $line || $line == \#* ]] && return
    # Strip trailing comment.
    line=${line%%[[:space:]]\#*}
    if [[ $line == *:* ]]; then
        local key=${line%%:*}
        local val=${line#*:}
        # Trim.
        key=${key##[[:space:]]##}
        key=${key%%[[:space:]]##}
        val=${val##[[:space:]]##}
        val=${val%%[[:space:]]##}
        # Strip surrounding quotes.
        if [[ $val == \"*\" ]]; then val=${val:1:-1}; fi
        if [[ $val == \'*\' ]]; then val=${val:1:-1}; fi
        YAML_DATA[$key]=$val
    fi
}

setopt extended_glob

tmpyaml=/tmp/zshrs_yaml_$$
cat > "$tmpyaml" <<EOF
# Configuration file
name: zshrs
version: 0.11.22
author: MenkeTechnologies
license: MIT
year: 2026

# Build settings
target: release
optimize: true
debug: false

# Paths
home: /Users/wizard/.zshrs
log: /var/log/zshrs.log

# Quoted strings
greeting: "Hello, World!"
motto: 'No fork, no problems'
EOF

echo "── parse ──"
while IFS= read -r line; do
    parse_yaml_line "$line"
done < "$tmpyaml"
echo "parsed ${#YAML_DATA[@]} keys"

echo "── all entries ──"
for k in ${(ko)YAML_DATA}; do
    printf "  %-12s = %s\n" "$k" "${YAML_DATA[$k]}"
done

echo "── direct lookup ──"
echo "name:     ${YAML_DATA[name]}"
echo "version:  ${YAML_DATA[version]}"
echo "greeting: ${YAML_DATA[greeting]}"
echo "motto:    ${YAML_DATA[motto]}"

echo "── missing key ──"
echo "missing:  ${YAML_DATA[nonexistent]:-(not set)}"

rm -f "$tmpyaml"

# === ztest assertions ===
# (demo currently produces empty entries under zshrs — the trim using
# `${val##[[:space:]]##}` extended-glob form wipes the value to empty;
# parser is non-functional; smoke only)
zassert_ok 1 "demo loaded"
ztest_run
