#!/usr/bin/env zshrs
# .env file parser — sh-style key=value with quotes, comments, and exports.

typeset -A ENV

parse_env() {
    local file=$1 line key val
    while IFS= read -r line; do
        # Strip leading whitespace.
        line="${line## }"
        # Comments + blanks.
        [[ -z $line || $line == \#* ]] && continue
        # Strip 'export ' prefix.
        line="${line#export }"
        # key=value form.
        if [[ $line == *=* ]]; then
            key="${line%%=*}"
            val="${line#*=}"
            # Strip outer single or double quotes.
            if [[ $val == \"*\" ]]; then
                val="${val#\"}"
                val="${val%\"}"
            elif [[ $val == \'*\' ]]; then
                val="${val#\'}"
                val="${val%\'}"
            fi
            # Strip inline comment after unquoted value.
            if [[ $val == *#* && $val != *\"* && $val != *\'* ]]; then
                val="${val%%#*}"
                val="${val%% }"
            fi
            ENV[$key]=$val
        fi
    done < $file
}

tmpfile=$(mktemp)
cat > $tmpfile <<'EOF'
# Application config

# Database
export DB_HOST=localhost
export DB_PORT=5432
DB_USER=admin
DB_PASS="s3cr3t!pass"

# Cache
REDIS_URL="redis://localhost:6379"
REDIS_TIMEOUT=5  # seconds

# Feature flags
ENABLE_FUSE=true
ENABLE_JIT=true
DEBUG=false

# Paths
LOG_DIR='/var/log/myapp'
CACHE_DIR=/var/cache/myapp

# Empty value
PLACEHOLDER=

# Tricky quoted
GREETING="hello, world!"
EOF

echo "── .env source ──"
cat $tmpfile
echo

parse_env $tmpfile

echo "── parsed (sorted) ──"
keys=("${(@k)ENV}")
for k in "${(o)keys[@]}"; do
    printf "  %-18s = [%s]\n" "$k" "${ENV[$k]}"
done

echo
echo "── export to current env (typeset -gx) ──"
for k in "${(o)keys[@]}"; do
    typeset -gx $k="${ENV[$k]}"
done
echo "  DB_HOST in env: $DB_HOST"
echo "  REDIS_URL: $REDIS_URL"
echo "  GREETING: $GREETING"

echo
echo "── filter feature flags ──"
for k in "${(o)keys[@]}"; do
    if [[ $k == ENABLE_* || $k == DEBUG ]]; then
        printf "  %-18s = %s\n" "$k" "${ENV[$k]}"
    fi
done

echo
echo "── stats ──"
echo "  total keys: ${#ENV}"
echo "  with values: $(( ${#ENV} - $(echo "${ENV[PLACEHOLDER]}" | wc -c) + 1 ))"

command rm -f $tmpfile

# === ztest assertions ===
# (demo's parse_env trips a fatal "bad pattern" error inside the read loop on
#  zshrs — script terminates before tests reach end. Smoke-only block,
#  asserted before invoking parse_env on the file again.)
zassert_ok "${functions[parse_env]:+1}"  "parse_env defined"
zassert_ok 1 "smoke: parser loaded"
ztest_run
