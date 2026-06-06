#!/usr/bin/env zshrs
# URL encode/decode in pure zsh.

url_encode() {
    local s=$1 i ch out=""
    for ((i = 1; i <= ${#s}; i++)); do
        ch=${s[i]}
        case $ch in
            [a-zA-Z0-9._~-]) out+=$ch ;;
            ' ') out+="+" ;;
            *)
                printf -v hex "%%%02X" "'$ch"
                out+=$hex
                ;;
        esac
    done
    echo "$out"
}

url_decode() {
    local s=$1 i ch out=""
    for ((i = 1; i <= ${#s}; i++)); do
        ch=${s[i]}
        case $ch in
            '+') out+=" " ;;
            '%')
                local hex="${s[i+1]}${s[i+2]}"
                out+=$(printf "\\x$hex")
                (( i += 2 ))
                ;;
            *) out+=$ch ;;
        esac
    done
    echo "$out"
}

echo "── encode ──"
for s in "hello world" "foo@bar.com" "search?q=hello&lang=en" "100% done"; do
    enc=$(url_encode "$s")
    echo "'$s' → '$enc'"
done

echo "── decode (round-trip) ──"
for s in "hello world" "foo@bar.com" "search?q=zsh%20shell"; do
    enc=$(url_encode "$s")
    dec=$(url_decode "$enc")
    if [[ "$s" == "$dec" ]]; then
        echo "OK: '$s'"
    else
        echo "FAIL: '$s' -> enc='$enc' -> dec='$dec'"
    fi
done

echo "── building query strings ──"
build_query() {
    local first=1 k v out=""
    while (( $# > 0 )); do
        k=$1; v=$2; shift 2
        if (( first )); then first=0; else out+="&"; fi
        out+="$(url_encode "$k")=$(url_encode "$v")"
    done
    echo "$out"
}

build_query name "Alice" "age" 30 "role" "ad/min"
build_query q "hello world" "lang" "en US"
build_query email "user@host.com" "subject" "100% off"

# === ztest assertions ===
zassert_eq "$(url_encode 'hello world')"   "hello+world"         "encode space → +"
zassert_eq "$(url_encode 'foo@bar.com')"   "foo%40bar.com"       "encode @"
zassert_eq "$(url_encode '100% done')"     "100%25+done"         "encode % and space"
zassert_eq "$(url_encode 'a.b-c_d~e')"     "a.b-c_d~e"           "encode preserves unreserved"
# Round-trip for safe inputs
for s in "hello world" "foo@bar.com" "abc 123"; do
    enc=$(url_encode "$s")
    dec=$(url_decode "$enc")
    zassert_eq "$dec" "$s" "round-trip: $s"
done
# build_query produces & separator
q=$(build_query a 1 b 2)
zassert_eq "$q"  "a=1&b=2"  "build_query a=1&b=2"
ztest_run
