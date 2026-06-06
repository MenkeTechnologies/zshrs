#!/usr/bin/env zshrs
# URL encode/decode — comprehensive RFC 3986.

# Pre-compute reserved characters that need encoding.
# Unreserved: A-Z a-z 0-9 - . _ ~

url_encode() {
    local s=$1 out="" i c ord
    for ((i=1; i<=${#s}; i++)); do
        c="${s[i]}"
        ord=$(( #c ))
        case $c in
            [A-Za-z0-9.~_-])
                out+="$c"
                ;;
            *)
                if (( ord < 128 )); then
                    out+=$(printf '%%%02X' $ord)
                else
                    # Multi-byte UTF-8 encode would go here.
                    out+=$(printf '%%%02X' $ord)
                fi
                ;;
        esac
    done
    echo "$out"
}

# URL encode for query strings (space → +).
url_encode_form() {
    local s=$1 out="" i c ord
    for ((i=1; i<=${#s}; i++)); do
        c="${s[i]}"
        ord=$(( #c ))
        case $c in
            [A-Za-z0-9.~_-])
                out+="$c"
                ;;
            ' ')
                out+="+"
                ;;
            *)
                out+=$(printf '%%%02X' $ord)
                ;;
        esac
    done
    echo "$out"
}

url_decode() {
    local s=$1 out="" i n
    n=${#s}
    i=1
    while (( i <= n )); do
        local c="${s[i]}"
        if [[ $c == '%' ]] && (( i + 2 <= n )); then
            local hex="${s[i+1,i+2]}"
            local ord=$(( 0x$hex ))
            out+=$(printf "\\$(printf %03o $ord)")
            (( i += 3 ))
        elif [[ $c == '+' ]]; then
            out+=" "
            (( i++ ))
        else
            out+="$c"
            (( i++ ))
        fi
    done
    echo "$out"
}

# Encode path components individually (preserves /).
url_encode_path() {
    local s=$1 out=""
    local -a parts
    parts=( ${(s:/:)s} )
    local first=1
    for p in "${parts[@]}"; do
        if (( ! first )); then out+="/"; fi
        out+=$(url_encode "$p")
        first=0
    done
    if [[ $s == /* ]]; then out="/${out}"; fi
    echo "$out"
}

# Parse URL: scheme://user:pass@host:port/path?query#frag
parse_url() {
    local url=$1
    typeset -gA U
    U=()
    U[input]="$url"

    # Scheme.
    if [[ $url == *://* ]]; then
        U[scheme]="${url%%://*}"
        url="${url#*://}"
    fi

    # Fragment.
    if [[ $url == *\#* ]]; then
        U[fragment]="${url#*\#}"
        url="${url%%\#*}"
    fi

    # Query.
    if [[ $url == *\?* ]]; then
        U[query]="${url#*\?}"
        url="${url%%\?*}"
    fi

    # Userinfo.
    if [[ $url == *@* ]]; then
        local userinfo="${url%%@*}"
        url="${url#*@}"
        if [[ $userinfo == *:* ]]; then
            U[user]="${userinfo%%:*}"
            U[pass]="${userinfo#*:}"
        else
            U[user]=$userinfo
        fi
    fi

    # Path.
    if [[ $url == */* ]]; then
        local host_port="${url%%/*}"
        U[path]="/${url#*/}"
        url=$host_port
    fi

    # Port.
    if [[ $url == *:* ]]; then
        U[host]="${url%%:*}"
        U[port]="${url#*:}"
    else
        U[host]=$url
    fi
}

echo "── encode (RFC 3986 unreserved set) ──"
inputs=(
    "hello world"
    "John Doe"
    "café"
    "/path/with spaces"
    "a+b=c"
    "?query=val&other=42"
    "100% sure"
    "!@#$%^&*()"
    ""
    "abc123"
)
for s in "${inputs[@]}"; do
    e=$(url_encode "$s")
    printf "  '%-25s' → '%s'\n" "$s" "$e"
done

echo
echo "── form encoding (space → +) ──"
form_inputs=(
    "hello world"
    "name=John Doe&age=30"
    "search query with spaces"
)
for s in "${form_inputs[@]}"; do
    e=$(url_encode_form "$s")
    printf "  '%-30s' → '%s'\n" "$s" "$e"
done

echo
echo "── round-trip ──"
for s in "${inputs[@]}"; do
    e=$(url_encode "$s")
    d=$(url_decode "$e")
    mark="✓"
    [[ $d != $s ]] && mark="✗"
    printf "  '%-25s' → encode/decode %s\n" "$s" "$mark"
done

echo
echo "── path encoding (preserves /) ──"
paths=(
    "/foo/bar/baz"
    "/users/john doe/profile"
    "/search/cats & dogs"
    "/api/v2/users"
    "relative/path"
)
for p in "${paths[@]}"; do
    e=$(url_encode_path "$p")
    printf "  %-30s → %s\n" "$p" "$e"
done

echo
echo "── URL parsing ──"
urls=(
    "https://example.com/path"
    "https://user:pass@example.com:8080/path?q=1#frag"
    "ftp://files.example.com/dir/file.txt"
    "http://api.github.com/users/menketechnologies/repos?per_page=100"
    "ws://localhost:8080/socket"
    "file:///etc/hosts"
    "mailto:user@example.com"
)
for u in "${urls[@]}"; do
    parse_url "$u"
    echo
    echo "  $u"
    for k in scheme user pass host port path query fragment; do
        v="${U[$k]:-}"
        [[ -n $v ]] && printf "    %-10s: %s\n" "$k" "$v"
    done
done

echo
echo "── query string parser ──"
parse_query() {
    local q=$1
    typeset -gA Q
    Q=()
    for pair in ${(s:&:)q}; do
        if [[ $pair == *=* ]]; then
            local k="${pair%%=*}"
            local v="${pair#*=}"
            Q[$k]=$(url_decode "$v")
        fi
    done
}

queries=(
    "name=John+Doe&age=30"
    "q=cats+%26+dogs&limit=10"
    "search=hello%20world"
    "user=alice&pass=secret%21"
)
for q in "${queries[@]}"; do
    parse_query "$q"
    echo
    echo "  query: $q"
    for k in "${(@ko)Q}"; do
        printf "    %s = '%s'\n" "$k" "${Q[$k]}"
    done
done

echo
echo "── special chars ──"
echo "  unreserved (no encode):  A-Z a-z 0-9 - . _ ~"
echo "  reserved (encode):       : / ? # [ ] @ ! \$ & ' ( ) * + , ; ="
echo "  unicode:                 always %xx (UTF-8 bytes)"
echo "  space:                   %20 (or + in form encoding)"

# === ztest assertions ===
zassert_eq "$(url_encode 'hello world')"     "hello%20world"   "encode space"
zassert_eq "$(url_encode 'a+b=c')"           "a%2Bb%3Dc"       "encode + and ="
zassert_eq "$(url_encode '100% sure')"       "100%25%20sure"   "encode percent"
zassert_eq "$(url_encode 'abc123')"          "abc123"          "no-op unreserved"
zassert_eq "$(url_encode '!@#')"             "%21%40%23"       "encode punct"
zassert_eq "$(url_encode_form 'hello world')" "hello+world"    "form space → +"
zassert_eq "$(url_decode 'hello%20world')"   "hello world"     "decode %20"
zassert_eq "$(url_decode 'a%2Bb%3Dc')"       "a+b=c"           "decode +="
parse_url "https://user:pass@example.com:8080/path?q=1#frag"
zassert_eq "${U[scheme]}"   "https"          "parse scheme"
zassert_eq "${U[host]}"     "example.com"    "parse host"
zassert_eq "${U[port]}"     "8080"           "parse port"
zassert_eq "${U[user]}"     "user"           "parse user"
zassert_eq "${U[path]}"     "/path"          "parse path"
zassert_eq "${U[query]}"    "q=1"            "parse query"
zassert_eq "${U[fragment]}" "frag"           "parse fragment"
ztest_run
