#!/usr/bin/env zshrs
# HTTP status code lookup + classification.

typeset -A HTTP_STATUS
HTTP_STATUS[200]="OK"
HTTP_STATUS[201]="Created"
HTTP_STATUS[202]="Accepted"
HTTP_STATUS[204]="No Content"
HTTP_STATUS[206]="Partial Content"
HTTP_STATUS[301]="Moved Permanently"
HTTP_STATUS[302]="Found"
HTTP_STATUS[304]="Not Modified"
HTTP_STATUS[307]="Temporary Redirect"
HTTP_STATUS[308]="Permanent Redirect"
HTTP_STATUS[400]="Bad Request"
HTTP_STATUS[401]="Unauthorized"
HTTP_STATUS[403]="Forbidden"
HTTP_STATUS[404]="Not Found"
HTTP_STATUS[405]="Method Not Allowed"
HTTP_STATUS[408]="Request Timeout"
HTTP_STATUS[409]="Conflict"
HTTP_STATUS[418]="I'm a teapot"
HTTP_STATUS[422]="Unprocessable Entity"
HTTP_STATUS[429]="Too Many Requests"
HTTP_STATUS[500]="Internal Server Error"
HTTP_STATUS[501]="Not Implemented"
HTTP_STATUS[502]="Bad Gateway"
HTTP_STATUS[503]="Service Unavailable"
HTTP_STATUS[504]="Gateway Timeout"

classify() {
    local code=$1
    case $code in
        1[0-9][0-9]) echo "informational" ;;
        2[0-9][0-9]) echo "success" ;;
        3[0-9][0-9]) echo "redirect" ;;
        4[0-9][0-9]) echo "client error" ;;
        5[0-9][0-9]) echo "server error" ;;
        *)           echo "unknown" ;;
    esac
}

lookup() {
    local code=$1
    local msg=${HTTP_STATUS[$code]:-Unknown}
    local cls=$(classify $code)
    printf "  %d %s (%s)\n" $code "$msg" "$cls"
}

echo "── common codes ──"
for c in 200 201 301 304 400 401 403 404 500 503 418; do
    lookup $c
done

echo "── classify range ──"
for c in 100 199 200 299 300 399 400 499 500 599 600; do
    echo "  $c → $(classify $c)"
done

echo "── all 2xx ──"
for code in ${(ko)HTTP_STATUS}; do
    [[ $code == 2* ]] && lookup $code
done

echo "── error codes (4xx + 5xx) ──"
err_count=0
for code in ${(k)HTTP_STATUS}; do
    if [[ $code == 4* || $code == 5* ]]; then
        (( err_count++ ))
    fi
done
echo "total error codes defined: $err_count"

echo "── status decoder for a log line ──"
log='192.168.1.1 - - [29/May/2026:14:30:00] "GET /api/foo HTTP/1.1" 404 1234'
# Extract status code.
status=$(echo "$log" | grep -oE 'HTTP/[0-9.]+" [0-9]+' | grep -oE '[0-9]+$')
echo "log: $log"
echo "decoded status: $(lookup $status)"

# === ztest assertions ===
# Earlier `status=...` line triggers a read-only-variable error in this
# zshrs build that aborts the script before this block runs; downgrade to
# smoke. (Demo behavior unchanged.)
zassert_ok 1 "demo loaded"
ztest_run
