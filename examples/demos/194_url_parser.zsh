#!/usr/bin/env zshrs
# URL parser — break URL into scheme/host/port/path/query/fragment.

parse_url() {
    local url=$1
    local scheme="" host="" port="" path="/" query="" fragment="" user="" pass=""

    # Fragment
    if [[ $url == *#* ]]; then
        fragment=${url##*#}
        url=${url%%#*}
    fi
    # Query
    if [[ $url == *\?* ]]; then
        query=${url##*\?}
        url=${url%%\?*}
    fi
    # Scheme
    if [[ $url == *://* ]]; then
        scheme=${url%%://*}
        url=${url#*://}
    fi
    # Path
    if [[ $url == */* ]]; then
        path=/${url#*/}
        url=${url%%/*}
    fi
    # User:pass@host
    if [[ $url == *@* ]]; then
        local userinfo=${url%%@*}
        url=${url#*@}
        if [[ $userinfo == *:* ]]; then
            user=${userinfo%%:*}
            pass=${userinfo#*:}
        else
            user=$userinfo
        fi
    fi
    # Host:port
    if [[ $url == *:* ]]; then
        host=${url%%:*}
        port=${url#*:}
    else
        host=$url
    fi

    echo "  scheme:   $scheme"
    echo "  user:     ${user:-(none)}"
    echo "  pass:     ${pass:-(none)}"
    echo "  host:     $host"
    echo "  port:     ${port:-(default)}"
    echo "  path:     $path"
    echo "  query:    ${query:-(none)}"
    echo "  fragment: ${fragment:-(none)}"
}

echo "── simple URL ──"
parse_url "https://example.com/path/to/resource"

echo "── with port ──"
parse_url "https://example.com:8080/api/v1"

echo "── with query ──"
parse_url "https://example.com/search?q=hello&lang=en"

echo "── with fragment ──"
parse_url "https://example.com/docs#section-2"

echo "── with userinfo ──"
parse_url "ftp://user:pass@ftp.example.com/files"

echo "── full URL ──"
parse_url "https://user:secret@api.example.com:443/v1/items?filter=active&sort=desc#results"

echo "── parse and rebuild query string ──"
rebuild_query() {
    local query=$1
    echo "  parsed:"
    local -a pairs=( ${(s/&/)query} )
    for kv in "${pairs[@]}"; do
        local k=${kv%%=*}
        local v=${kv#*=}
        printf "    %-10s = %s\n" "$k" "$v"
    done
}
rebuild_query "name=alice&age=30&role=admin&active=true"

echo "── extract just hostname ──"
extract_host() {
    local url=$1
    url=${url#*://}
    url=${url%%/*}
    url=${url%%:*}
    url=${url#*@}
    echo "$url"
}
for u in "https://www.example.com/path" "http://10.0.0.1:80/" "ftp://user@ftp.example.org/"; do
    echo "  $u → $(extract_host $u)"
done
