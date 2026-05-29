#!/usr/bin/env zshrs
# compdef-style completion definitions — config form only (no actual
# tab-firing since we're non-interactive).
# Ported from Src/Zle/compsys.c + compinit infrastructure.

# In zsh you'd normally `autoload -Uz compinit && compinit` first.
# Skip that here; just exercise the configuration API.

echo "── define a compdef-style completion fn ──"
_my_app() {
    local -a opts
    opts=(
        "-h:show help"
        "--help:show help long"
        "-v:verbose"
        "-V:version"
        "-o:output file"
    )
    # In a real fn this would call _arguments / _describe.
    echo "my_app completion fn defined (args: $#words)"
    echo "potential opts: ${opts[@]}"
}

echo "── register via compdef-style (if available) ──"
if command -v compdef >/dev/null 2>&1; then
    compdef _my_app my_app 2>/dev/null && echo "compdef registered" || echo "compdef requires compinit"
else
    echo "(compdef not loaded in this shell)"
fi

echo "── _arguments-like declaration pattern ──"
_describe_opts() {
    local desc=$1
    shift
    echo "── $desc ──"
    for spec in "$@"; do
        local opt=${spec%%:*}
        local help_text=${spec#*:}
        printf "  %-10s %s\n" "$opt" "$help_text"
    done
}

_describe_opts "my-app" \
    "-h:Show help" \
    "-v:Verbose output" \
    "-d:Debug mode" \
    "--config:Specify config file" \
    "--output:Write to file"

echo "── _values pattern (key=value table) ──"
_describe_kvs() {
    local desc=$1
    shift
    echo "── $desc ──"
    while (( $# >= 2 )); do
        printf "  %-12s = %s\n" "$1" "$2"
        shift 2
    done
}

_describe_kvs "options" \
    debug    "1=on, 0=off" \
    log_file "path to log" \
    workers  "thread pool size" \
    timeout  "seconds"

echo "── manual completion candidate generator ──"
generate_candidates() {
    local prefix=$1
    shift
    local candidate
    for candidate in "$@"; do
        if [[ $candidate == ${prefix}* ]]; then
            echo "  - $candidate"
        fi
    done
}

candidates=(alpha beta gamma alphabet alligator beam zebra)
echo "matches 'a':"
generate_candidates a "${candidates[@]}"
echo "matches 'al':"
generate_candidates al "${candidates[@]}"
echo "matches 'b':"
generate_candidates b "${candidates[@]}"
