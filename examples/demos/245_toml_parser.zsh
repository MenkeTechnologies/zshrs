#!/usr/bin/env zshrs
# TOML parser — minimal: sections, key=value, strings, ints, bools, arrays.

typeset -A TOML

parse_toml() {
    local file=$1 section="" line key val
    while IFS= read -r line; do
        # Strip comments and trailing whitespace.
        line="${line%%#*}"
        line="${line%% }"
        line="${line## }"
        # Skip blank.
        [[ -z $line ]] && continue
        # Section header.
        if [[ $line == \[*\] ]]; then
            section="${line#[}"
            section="${section%]}"
            continue
        fi
        # key = value.
        if [[ $line == *=* ]]; then
            key="${line%%=*}"
            val="${line#*=}"
            # Trim whitespace.
            key="${key%% }"; key="${key## }"
            val="${val## }"; val="${val%% }"
            # Strip outer quotes if string.
            if [[ $val == \"*\" ]]; then
                val="${val#\"}"
                val="${val%\"}"
            fi
            local full_key
            if [[ -n $section ]]; then
                full_key="${section}.${key}"
            else
                full_key="$key"
            fi
            TOML[$full_key]=$val
        fi
    done < $file
}

# Build a sample TOML.
tmpfile=$(mktemp)
cat > $tmpfile <<'EOF'
# Sample config
title = "zshrs config"
version = 4.7

[server]
host = "localhost"
port = 8080
enabled = true

[database]
name = "mydb"
pool = 16
timeout = 30

[features]
fuse = true
jit = true
parallel = true

[paths]
log = "/var/log/zshrs.log"
cache = "/var/cache/zshrs"
EOF

echo "── source TOML ──"
cat $tmpfile
echo

parse_toml $tmpfile

echo "── parsed (sorted) ──"
keys=("${(@k)TOML}")
for k in "${(o)keys[@]}"; do
    printf "  %-30s = %s\n" "$k" "${TOML[$k]}"
done

echo
echo "── accessors ──"
echo "  title:      ${TOML[title]}"
echo "  version:    ${TOML[version]}"
echo "  server:     ${TOML[server.host]}:${TOML[server.port]}"
echo "  db pool:    ${TOML[database.pool]}"
echo "  log path:   ${TOML[paths.log]}"

echo
echo "── filter by section prefix ──"
for prefix in server database features paths; do
    echo "[$prefix]"
    for k in "${(o)keys[@]}"; do
        if [[ $k == ${prefix}.* ]]; then
            local short="${k#${prefix}.}"
            printf "  %-12s = %s\n" "$short" "${TOML[$k]}"
        fi
    done
done

command rm -f $tmpfile

# === ztest assertions ===
zassert_eq "${TOML[title]}"          "zshrs config"        "title parsed"
zassert_eq "${TOML[version]}"        "4.7"                 "version parsed"
zassert_eq "${TOML[server.host]}"    "localhost"           "section.key host"
zassert_eq "${TOML[server.port]}"    "8080"                "section.key port"
zassert_eq "${TOML[server.enabled]}" "true"                "bool stored as string"
zassert_eq "${TOML[database.name]}"  "mydb"                "db.name"
zassert_eq "${TOML[database.pool]}"  "16"                  "db.pool"
zassert_eq "${TOML[features.fuse]}"  "true"                "features.fuse"
zassert_eq "${TOML[paths.log]}"      "/var/log/zshrs.log"  "paths.log"
zassert_eq "${TOML[paths.cache]}"    "/var/cache/zshrs"    "paths.cache"
zassert_ge "${#TOML[@]}" "12" "at least 12 keys parsed"
ztest_run
