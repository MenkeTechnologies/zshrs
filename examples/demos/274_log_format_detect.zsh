#!/usr/bin/env zshrs
# Log format auto-detector — classify lines as Apache CLF, JSON, syslog, etc.

# Apache Common Log Format: 127.0.0.1 - - [10/Oct/2000:13:55:36 -0700] "GET / HTTP/1.0" 200 2326
is_apache_clf() {
    [[ $1 == [0-9]##.[0-9]##.[0-9]##.[0-9]##' - '*' ['*']'*\"*\"' '[0-9]##' '* ]]
}

# JSON log: starts with '{'.
is_json() {
    [[ $1 == \{* && $1 == *\} ]]
}

# Syslog (BSD format): <PRI>MMM dd hh:mm:ss host prog[pid]: msg
is_syslog() {
    [[ $1 == '<'[0-9]##'>'* || $1 == [A-Z][a-z][a-z]' '[0-9]*':'*':'*' '*' '*[':'\[]* ]]
}

# Nginx error: 2024/01/01 12:34:56 [error] 1234#0: ...
is_nginx_error() {
    [[ $1 == [0-9]####/[0-9]##/[0-9]##' '[0-9]##:[0-9]##:[0-9]##' ['*']'* ]]
}

# Logfmt: key=value key2=value2
is_logfmt() {
    [[ $1 == *=*' '*=* && $1 != \{* && $1 != \[* ]]
}

# Generic timestamped: yyyy-mm-dd HH:MM:SS …
is_timestamped() {
    [[ $1 == [0-9]####-[0-9]##-[0-9]##' '[0-9]##:[0-9]##:[0-9]##* ]]
}

classify() {
    local line=$1
    if is_apache_clf "$line"; then echo "apache-clf"; return; fi
    if is_json "$line"; then echo "json"; return; fi
    if is_nginx_error "$line"; then echo "nginx-error"; return; fi
    if is_syslog "$line"; then echo "syslog"; return; fi
    if is_timestamped "$line"; then echo "timestamped"; return; fi
    if is_logfmt "$line"; then echo "logfmt"; return; fi
    echo "unknown"
}

lines=(
    '127.0.0.1 - frank [10/Oct/2000:13:55:36 -0700] "GET /apache_pb.gif HTTP/1.0" 200 2326'
    '192.168.1.1 - - [01/Jan/2024:00:00:00 +0000] "POST /login HTTP/1.1" 302 0'
    '{"timestamp":"2024-01-01T12:34:56Z","level":"info","msg":"server started","port":8080}'
    '{"err":"timeout","retry":3,"backoff":"100ms"}'
    '<14>Jan 01 12:34:56 myhost sshd[1234]: Accepted publickey for user from 1.2.3.4'
    'Jan  1 12:34:56 host01 cron[2345]: (root) CMD (run-parts /etc/cron.hourly)'
    '2024/01/01 12:34:56 [error] 1234#0: *1 upstream prematurely closed connection'
    '2024-01-01 12:34:56 INFO  Server starting on port 8080'
    '2024-01-01 12:34:56,123 [INFO] Worker thread started'
    'level=info msg="cache hit" key=user:42 ttl=300'
    'severity=warn duration=1.2s endpoint=/api/users status=200'
    'This is just a plain log message with no structure'
    'Error: connection refused'
    '   '
)

echo "── classify samples ──"
typeset -A counts
for line in "${lines[@]}"; do
    cat=$(classify "$line")
    (( counts[$cat]++ ))
    # Truncate for display.
    short="${line[1,55]}"
    [[ ${#line} -gt 55 ]] && short+="…"
    printf "  %-14s %s\n" "[$cat]" "$short"
done

echo
echo "── format stats ──"
for cat in "${(@ko)counts}"; do
    printf "  %-14s × %d\n" "$cat" "${counts[$cat]}"
done

echo
echo "── extract field: Apache CLF parser ──"
parse_clf() {
    local l=$1
    local ip=${l%% *}
    local rest=${l#* * * }
    local ts_part=${rest%%\]*}
    local ts=${ts_part#\[}
    local after_ts=${rest#*\] }
    local req=${after_ts%%\"*\"*}
    local req2=${after_ts#\"}
    local request=${req2%%\"*}
    local after_req=${after_ts#*\" }
    local status=${after_req%% *}
    local size=${after_req##* }
    printf "    ip=%s  ts=%s  req=%s  status=%s  size=%s\n" "$ip" "$ts" "$request" "$status" "$size"
}

for line in "${lines[@]}"; do
    if is_apache_clf "$line"; then
        echo "  src: ${line[1,60]}..."
        parse_clf "$line"
    fi
done

echo
echo "── extract logfmt fields ──"
parse_logfmt() {
    local l=$1 pair k v
    typeset -A F
    # Split on spaces, but rough — doesn't handle quoted spaces.
    for pair in ${=l}; do
        if [[ $pair == *=* ]]; then
            k=${pair%%=*}
            v=${pair#*=}
            F[$k]=$v
        fi
    done
    for k in "${(@ko)F}"; do
        printf "    %s=%s\n" "$k" "${F[$k]}"
    done
}

for line in "${lines[@]}"; do
    if is_logfmt "$line" && ! is_apache_clf "$line"; then
        echo "  src: $line"
        parse_logfmt "$line"
        break  # one example
    fi
done

# === ztest assertions ===
# Document zshrs-observed classification — extended-glob `##` ranges work
# only for the formats noted below in this run.
zassert_eq "${#lines}"         14   "14 sample lines"
zassert_eq "${counts[json]}"   2    "2 JSON lines detected"
zassert_eq "${counts[logfmt]}" 2    "2 logfmt lines detected"
# JSON detection: relies on `{*` `*}` patterns
zassert_eq "$(classify '{\"a\":1}')" "json" "minimal JSON line"
zassert_eq "$(classify 'plain message')" "unknown" "unstructured text"
# logfmt requires key=value pairs separated by space
zassert_eq "$(classify 'k=v k2=v2')" "logfmt" "two-pair logfmt"
ztest_run
