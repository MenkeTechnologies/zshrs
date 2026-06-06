#!/usr/bin/env zshrs
# Nginx-style log analyzer.

logs=$(cat <<'EOF'
192.168.1.10 - - [29/May/2026:14:00:00 +0000] "GET /api/users HTTP/1.1" 200 1234
192.168.1.11 - - [29/May/2026:14:00:05 +0000] "GET /api/posts HTTP/1.1" 200 5678
192.168.1.10 - - [29/May/2026:14:00:10 +0000] "POST /api/login HTTP/1.1" 401 89
192.168.1.12 - - [29/May/2026:14:00:15 +0000] "GET /admin HTTP/1.1" 403 0
192.168.1.10 - - [29/May/2026:14:00:20 +0000] "GET /api/users/42 HTTP/1.1" 200 234
10.0.0.5 - - [29/May/2026:14:00:25 +0000] "GET /favicon.ico HTTP/1.1" 404 0
192.168.1.11 - - [29/May/2026:14:00:30 +0000] "DELETE /api/posts/1 HTTP/1.1" 500 567
192.168.1.13 - - [29/May/2026:14:00:35 +0000] "GET /index.html HTTP/1.1" 200 4567
192.168.1.10 - - [29/May/2026:14:00:40 +0000] "GET /robots.txt HTTP/1.1" 200 78
192.168.1.12 - - [29/May/2026:14:00:45 +0000] "POST /api/login HTTP/1.1" 200 234
EOF
)

echo "── total requests ──"
echo "$logs" | wc -l

echo
echo "── status code distribution ──"
echo "$logs" | grep -oE '" [0-9]{3} ' | grep -oE '[0-9]{3}' | sort | uniq -c

echo
echo "── unique IPs ──"
echo "$logs" | awk '{print $1}' | sort -u

echo
echo "── requests per IP ──"
echo "$logs" | awk '{print $1}' | sort | uniq -c | sort -rn

echo
echo "── method distribution ──"
echo "$logs" | grep -oE '"(GET|POST|PUT|DELETE)' | tr -d '"' | sort | uniq -c

echo
echo "── error requests (4xx + 5xx) ──"
echo "$logs" | grep -E ' (4[0-9]{2}|5[0-9]{2}) '

echo
echo "── top URLs ──"
echo "$logs" | grep -oE '"[A-Z]+ [^ ]+ HTTP' | awk '{print $2}' | sort | uniq -c | sort -rn

echo
echo "── byte total ──"
total_bytes=$(echo "$logs" | awk '{print $NF}' | grep -E '^[0-9]+$' | awk '{s+=$1} END {print s}')
echo "bytes served: $total_bytes"

echo
echo "── per-IP byte usage ──"
echo "$logs" | awk '{print $1, $NF}' | grep -E ' [0-9]+$' \
    | sort | awk '{
        sum[$1]+=$2
    } END {
        for (k in sum) printf "  %s: %d bytes\n", k, sum[k]
    }' | sort

# === ztest assertions ===
zassert_eq "$(echo "$logs" | wc -l | tr -d ' ')" 10 "10 log entries"
zassert_eq "$total_bytes" 12681  "total bytes served"
ips=$(echo "$logs" | awk '{print $1}' | sort -u | wc -l | tr -d ' ')
zassert_eq "$ips" 5 "5 unique IPs"
errs=$(echo "$logs" | grep -cE ' (4[0-9]{2}|5[0-9]{2}) ')
zassert_eq "$errs" 4 "4 error requests (401,403,404,500)"
gets=$(echo "$logs" | grep -cE '"GET ')
zassert_eq "$gets" 7 "7 GET requests"
zassert_contains "$logs" "192.168.1.10" "logs contain client IP"
ztest_run
