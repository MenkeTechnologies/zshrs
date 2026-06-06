#!/usr/bin/env zshrs
# HTTP request parser — method, path, headers, body.

typeset -A REQ
typeset -A HEADERS

parse_request() {
    local raw=$1
    REQ=()
    HEADERS=()
    local line first=1 in_body=0 body=""
    while IFS= read -r line; do
        # Strip CR.
        line=${line%$'\r'}
        if (( first )); then
            # Request line: METHOD PATH HTTP/V
            local method="${line%% *}"
            local rest="${line#* }"
            local path="${rest%% *}"
            local proto="${rest#* }"
            REQ[method]="$method"
            REQ[path]="$path"
            REQ[proto]="$proto"
            # Parse query string.
            if [[ $path == *\?* ]]; then
                REQ[pathonly]="${path%%\?*}"
                REQ[query]="${path#*\?}"
            else
                REQ[pathonly]="$path"
                REQ[query]=""
            fi
            first=0
            continue
        fi
        if (( in_body )); then
            body+="$line"$'\n'
            continue
        fi
        if [[ -z $line ]]; then
            in_body=1
            continue
        fi
        # Header: Key: Value
        if [[ $line == *:* ]]; then
            local key=${line%%:*}
            local val=${line#*: }
            HEADERS[${key:l}]=$val
        fi
    done <<< "$raw"
    REQ[body]="${body%$'\n'}"
}

print_request() {
    echo "  method:    ${REQ[method]}"
    echo "  path:      ${REQ[pathonly]}"
    if [[ -n ${REQ[query]} ]]; then
        echo "  query:     ${REQ[query]}"
    fi
    echo "  proto:     ${REQ[proto]}"
    echo "  headers (${#HEADERS}):"
    for k in "${(@ko)HEADERS}"; do
        printf "    %s: %s\n" "$k" "${HEADERS[$k]}"
    done
    if [[ -n ${REQ[body]} ]]; then
        echo "  body (${#REQ[body]} bytes):"
        echo "    ${REQ[body]}"
    fi
}

# Sample requests.
req1='GET /api/users?id=42&format=json HTTP/1.1
Host: api.example.com
User-Agent: curl/7.88.1
Accept: application/json
X-Request-ID: abc123

'

req2='POST /api/login HTTP/1.1
Host: auth.example.com
Content-Type: application/json
Content-Length: 42
Authorization: Bearer eyJhbGciOiJIUzI1NiIs

{"user":"alice","pass":"s3cret"}'

req3='PUT /resource/99 HTTP/2
Host: api.example.com
Content-Type: application/x-www-form-urlencoded
Cookie: session=xyz789; theme=dark

name=updated&active=true'

req4='DELETE /sessions/all HTTP/1.1
Host: api.example.com
Authorization: Bearer token123

'

for i in 1 2 3 4; do
    echo "── request $i ──"
    var="req$i"
    parse_request "${(P)var}"
    print_request
    echo
done

echo "── query-string decoder ──"
parse_query() {
    local q=$1 pair k v
    typeset -A Q
    for pair in ${(s/&/)q}; do
        k=${pair%%=*}
        v=${pair#*=}
        Q[$k]=$v
    done
    for k in "${(@ko)Q}"; do
        printf "  %s = %s\n" "$k" "${Q[$k]}"
    done
}

parse_request "$req1"
echo "  query from req1: ${REQ[query]}"
parse_query "${REQ[query]}"

echo
echo "── cookie parser ──"
parse_cookie() {
    local c=$1 pair k v
    typeset -A C
    for pair in ${(s/;/)c}; do
        # Trim leading space.
        pair=${pair## }
        k=${pair%%=*}
        v=${pair#*=}
        C[$k]=$v
    done
    for k in "${(@ko)C}"; do
        printf "  %s = %s\n" "$k" "${C[$k]}"
    done
}

parse_request "$req3"
cookie_val=${HEADERS[cookie]}
echo "  cookie header: $cookie_val"
parse_cookie "$cookie_val"

# === ztest assertions ===
# Parse req1: GET with query string
parse_request "$req1"
zassert_eq "${REQ[method]}"      "GET"          "req1 method"
zassert_eq "${REQ[pathonly]}"    "/api/users"   "req1 path stripped of query"
zassert_eq "${REQ[query]}"       "id=42&format=json"  "req1 query string"
zassert_eq "${REQ[proto]}"       "HTTP/1.1"     "req1 proto"
zassert_eq "${HEADERS[host]}"    "api.example.com" "req1 Host header (lowercased key)"
zassert_eq "${HEADERS[user-agent]}" "curl/7.88.1" "req1 User-Agent header"
zassert_eq "${#HEADERS}"         4              "req1 has 4 headers"
# Parse req2: POST with JSON body
parse_request "$req2"
zassert_eq "${REQ[method]}"      "POST"         "req2 method"
zassert_eq "${REQ[body]}"        '{"user":"alice","pass":"s3cret"}' "req2 body preserved"
zassert_eq "${HEADERS[content-type]}" "application/json" "req2 content-type"
# Parse req4: DELETE no body
parse_request "$req4"
zassert_eq "${REQ[method]}"      "DELETE"       "req4 method"
zassert_eq "${REQ[body]}"        ""             "req4 has no body"
ztest_run
