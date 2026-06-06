#!/usr/bin/env zshrs
# ANSI escape sequence stripper — render colored output as plain text.

strip_ansi() {
    local s=$1
    # ESC = \033 = 0x1B. Pattern: ESC [ ... letter
    local out="" i ch
    local in_esc=0
    for ((i=1; i<=${#s}; i++)); do
        ch=${s[i]}
        if (( in_esc )); then
            # End of CSI sequence is a letter.
            case $ch in
                [a-zA-Z]) in_esc=0 ;;
            esac
            continue
        fi
        if [[ $ch == $'\x1b' ]]; then
            # Look for '['
            local next=${s[i+1]}
            if [[ $next == "[" ]]; then
                in_esc=1
                (( i++ ))
                continue
            fi
        fi
        out+=$ch
    done
    echo "$out"
}

echo "── strip color codes ──"
colored=$'\x1b[31mred text\x1b[0m and \x1b[32mgreen text\x1b[0m'
echo "raw bytes: $(echo "$colored" | wc -c) chars"
plain=$(strip_ansi "$colored")
echo "stripped: '$plain' (${#plain} chars)"

echo "── multiple sequences ──"
multi=$'\x1b[1;31mbold red\x1b[0m and \x1b[4;33munderlined yellow\x1b[0m'
echo "raw:    $multi"
echo "plain:  $(strip_ansi "$multi")"

echo "── nested ──"
nested=$'pre \x1b[31m[\x1b[1mERR\x1b[0;31m]\x1b[0m post'
echo "raw:    $nested"
echo "plain:  $(strip_ansi "$nested")"

echo "── from a log line ──"
log=$'\x1b[33m[WARN]\x1b[0m something happened at \x1b[36m2026-05-29\x1b[0m'
echo "$log"
echo "→ $(strip_ansi "$log")"

echo "── length comparison ──"
for c in \
    $'\x1b[31mhello\x1b[0m' \
    $'\x1b[1mbold\x1b[0m' \
    $'plain' \
    $'\x1b[3;33;44mitalic yellow bg blue\x1b[0m'; do
    plain=$(strip_ansi "$c")
    printf "raw=%-30s len=%2d stripped='%s' len=%2d\n" "$c" "${#c}" "$plain" "${#plain}"
done

# === ztest assertions ===
zassert_eq "$(strip_ansi $'\x1b[31mred\x1b[0m')" "red"        "strip simple SGR"
zassert_eq "$(strip_ansi $'plain')" "plain"                    "passthrough plain text"
zassert_eq "$(strip_ansi $'\x1b[1;31mbold red\x1b[0m')" "bold red" "multi-arg SGR"
zassert_eq "$(strip_ansi $'pre \x1b[31m[\x1b[1mERR\x1b[0;31m]\x1b[0m post')" "pre [ERR] post" "nested SGR around brackets"
# raw colored 'hello' is 5 chars wrapped in 9 bytes of escapes = 14 chars total
raw=$'\x1b[31mhello\x1b[0m'
zassert_eq "${#raw}" 14   "raw length includes escapes"
zassert_eq "${#$(strip_ansi "$raw")}" 5  "stripped is 5 chars"
ztest_run
