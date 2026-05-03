# zshrs-daemon POSIX shell wrappers (dash / sh)
# =============================================
# POSIX-strict version of daemon-shell.zsh — runs in dash, BusyBox sh,
# and any /bin/sh implementation. Source from your ~/.profile:
#
#     . /path/to/daemon-shell.sh
#     daemon_ping
#     id=$(daemon_job_submit -- find / -name '*.sh' 2>/dev/null)
#     daemon_job_status "$id"
#
# IMPORTANT: function names use UNDERSCORES not hyphens. POSIX
# (and dash) only allow `[A-Za-z_][A-Za-z0-9_]*` identifiers.
# bash/zsh/ksh93/mksh/fish all accept hyphens — those wrappers
# (daemon-shell.{bash,zsh,ksh,fish}) keep the hyphen style for
# typing ergonomics. This file uses snake_case so it loads under
# dash without "Bad function name" errors.
#
# Verified shells: dash, mksh, BusyBox sh. macOS's stock ksh (very
# old AT&T ksh) silently mis-evaluates this file — use bash, mksh,
# or a real ksh93 build there.
#
# Setup ($HOME/.zshrs/daemon.toml):
#     [http]
#     listen = "127.0.0.1:7733"
#     # tokens = ["..."]    # required for non-loopback binds
#
# Env overrides:
#     export DAEMON_URL=http://127.0.0.1:7733
#     export DAEMON_TOKEN=long-random-secret
#     export DAEMON_SHELL_ID=dash      # see docs/SHELL_IDS.md

: ${DAEMON_URL:=http://127.0.0.1:7733}
: ${DAEMON_TOKEN:=}
: ${DAEMON_SHELL_ID:=dash}

_daemon_curl() {
    if [ -n "$DAEMON_TOKEN" ]; then
        curl -sS -f -H "Authorization: Bearer $DAEMON_TOKEN" "$@"
    else
        curl -sS -f "$@"
    fi
}

_daemon_post() {
    op=$1; shift
    body=${1:-'{}'}
    _daemon_curl \
        -H 'Content-Type: application/json' \
        --data-raw "$body" \
        "$DAEMON_URL/op/$op"
}

_daemon_get() {
    _daemon_curl "$DAEMON_URL$1"
}

# Minimal JSON-string escape — backslash, double-quote, newline, tab,
# CR. Anything else passes through. For binary data, base64 separately.
# awk-based so it works on BSD (macOS) and GNU sed environments alike;
# sed's `\1`/`\2` would be interpreted as capture-group back-references
# on BSD and reject the pattern. RS=0x01 makes awk treat the entire
# input as one record so embedded newlines don't terminate processing.
_json_str() {
    printf '%s\1' "$1" | awk 'BEGIN { RS="\1" } {
        gsub(/\\/, "\\\\")
        gsub(/"/, "\\\"")
        gsub(/\t/, "\\t")
        gsub(/\r/, "\\r")
        gsub(/\n/, "\\n")
        printf "%s", $0
        exit
    }'
}

# ---- Public commands ------------------------------------------------------

daemon_health() { _daemon_get /health; }
daemon_ops()    { _daemon_get /ops; }
daemon_info()   { _daemon_post info '{}'; }

daemon_ping() {
    if [ $# -gt 0 ]; then
        _daemon_post ping "{\"echo\":\"$*\"}"
    else
        _daemon_post ping '{}'
    fi
}

daemon_call() {
    if [ $# -lt 1 ]; then
        echo 'usage: daemon_call OP [JSON]' >&2
        return 2
    fi
    op=$1; shift
    body=${1:-'{}'}
    _daemon_post "$op" "$body"
}

# ---- Federated recorder (definitions.*) ----------------------------------

_daemon_emit() {
    # _daemon_emit KIND NAME [VALUE] [FILE] [LINE] [FN_CHAIN]
    kind=$1
    name=$2
    value=${3:-}
    file=${4:-}
    line=${5:-}
    chain=${6:-}
    body="{\"shell_id\":\"$DAEMON_SHELL_ID\",\"kind\":\"$kind\""
    body="$body,\"name\":\"$(_json_str "$name")\""
    [ -n "$value" ] && body="$body,\"value\":\"$(_json_str "$value")\""
    [ -n "$file" ]  && body="$body,\"file\":\"$(_json_str "$file")\""
    [ -n "$line" ]  && body="$body,\"line\":$line"
    [ -n "$chain" ] && body="$body,\"fn_chain\":\"$(_json_str "$chain")\""
    body="$body}"
    _daemon_post definitions_emit "$body"
}

daemon_record_alias()    { [ $# -lt 2 ] && { echo 'usage: daemon_record_alias NAME BODY' >&2; return 2; }; _daemon_emit alias "$1" "$2"; }
daemon_record_galias()   { [ $# -lt 2 ] && { echo 'usage: daemon_record_galias NAME BODY' >&2; return 2; }; _daemon_emit galias "$1" "$2"; }
daemon_record_salias()   { [ $# -lt 2 ] && { echo 'usage: daemon_record_salias NAME BODY' >&2; return 2; }; _daemon_emit salias "$1" "$2"; }
daemon_record_function() { [ $# -lt 2 ] && { echo 'usage: daemon_record_function NAME BODY' >&2; return 2; }; _daemon_emit function "$1" "$2"; }
daemon_record_export()   { [ $# -lt 2 ] && { echo 'usage: daemon_record_export NAME VALUE' >&2; return 2; }; _daemon_emit env "$1" "$2"; }
daemon_record_param()    { [ $# -lt 2 ] && { echo 'usage: daemon_record_param NAME VALUE' >&2; return 2; }; _daemon_emit params "$1" "$2"; }
daemon_record_bindkey()  { [ $# -lt 2 ] && { echo 'usage: daemon_record_bindkey SEQ WIDGET' >&2; return 2; }; _daemon_emit bindkey "$1" "$2"; }
daemon_record_compdef()  { [ $# -lt 2 ] && { echo 'usage: daemon_record_compdef CMD COMPLETER' >&2; return 2; }; _daemon_emit compdef "$1" "$2"; }
daemon_record_zstyle()   { [ $# -lt 2 ] && { echo 'usage: daemon_record_zstyle PATTERN STYLE' >&2; return 2; }; _daemon_emit zstyle "$1" "$2"; }
daemon_record_zmodload() { [ $# -lt 1 ] && { echo 'usage: daemon_record_zmodload MODULE' >&2; return 2; }; _daemon_emit zmodload "$1"; }
daemon_record_setopt()   { [ $# -lt 1 ] && { echo 'usage: daemon_record_setopt OPT' >&2; return 2; }; _daemon_emit setopt "$1" on; }
daemon_record_unsetopt() { [ $# -lt 1 ] && { echo 'usage: daemon_record_unsetopt OPT' >&2; return 2; }; _daemon_emit setopt "$1" off; }
daemon_record_source()   { [ $# -lt 1 ] && { echo 'usage: daemon_record_source PATH' >&2; return 2; }; _daemon_emit source "$1"; }
daemon_record_path()     { [ $# -lt 1 ] && { echo 'usage: daemon_record_path DIR' >&2; return 2; }; _daemon_emit path "$1"; }
daemon_record_fpath()    { [ $# -lt 1 ] && { echo 'usage: daemon_record_fpath DIR' >&2; return 2; }; _daemon_emit fpath "$1"; }
daemon_record_zle()      { [ $# -lt 1 ] && { echo 'usage: daemon_record_zle WIDGET [BODY]' >&2; return 2; }; _daemon_emit zle "$1" "${2:-}"; }
daemon_record_trap()     { [ $# -lt 2 ] && { echo 'usage: daemon_record_trap SIGNAL HANDLER' >&2; return 2; }; _daemon_emit trap "$1" "$2"; }
daemon_record_named_dir() { [ $# -lt 2 ] && { echo 'usage: daemon_record_named_dir NAME PATH' >&2; return 2; }; _daemon_emit named_dir "$1" "$2"; }
daemon_record_completion() { [ $# -lt 1 ] && { echo 'usage: daemon_record_completion CMD [PATH]' >&2; return 2; }; _daemon_emit completion "$1" "${2:-}"; }

# ---- Federated catalog query / diff --------------------------------------

# daemon_defs_query --kind K --name N --prefix P --shell-id S --limit N
daemon_defs_query() {
    kind=
    name=
    prefix=
    shell=
    limit=
    while [ $# -gt 0 ]; do
        case $1 in
            --kind)     kind=$2;   shift 2 ;;
            --name)     name=$2;   shift 2 ;;
            --prefix)   prefix=$2; shift 2 ;;
            --shell-id) shell=$2;  shift 2 ;;
            --limit)    limit=$2;  shift 2 ;;
            *) echo "unknown arg: $1" >&2; return 2 ;;
        esac
    done
    body='{'
    sep=
    [ -n "$kind" ]   && { body="$body${sep}\"kind\":\"$kind\""; sep=','; }
    [ -n "$name" ]   && { body="$body${sep}\"name\":\"$name\""; sep=','; }
    [ -n "$prefix" ] && { body="$body${sep}\"prefix\":\"$prefix\""; sep=','; }
    [ -n "$shell" ]  && { body="$body${sep}\"shell_id\":\"$shell\""; sep=','; }
    [ -n "$limit" ]  && { body="$body${sep}\"limit\":$limit"; sep=','; }
    body="$body}"
    _daemon_post definitions_query "$body"
}

daemon_defs_kinds() { _daemon_post definitions_kinds '{}'; }

# daemon_defs_diff SHELL_A SHELL_B [KIND]
daemon_defs_diff() {
    if [ $# -lt 2 ]; then
        echo 'usage: daemon_defs_diff SHELL_A SHELL_B [KIND]' >&2
        return 2
    fi
    body="{\"shell_a\":\"$1\",\"shell_b\":\"$2\""
    [ -n "${3:-}" ] && body="$body,\"kind\":\"$3\""
    body="$body}"
    _daemon_post definitions_diff "$body"
}

# ---- Streaming -----------------------------------------------------------

daemon_watch() {
    if [ $# -lt 1 ]; then
        echo 'usage: daemon_watch DIR [--recursive]' >&2
        return 2
    fi
    dir=$1
    recursive=false
    [ "${2:-}" = '--recursive' ] && recursive=true
    _daemon_curl -N "$DAEMON_URL/stream/watch?path=$dir&recursive=$recursive"
}

daemon_events() {
    pat=${1:-'*.*'}
    _daemon_curl -N "$DAEMON_URL/stream/events?channel=$pat"
}

daemon_publish() {
    if [ $# -ne 2 ]; then
        echo 'usage: daemon_publish TOPIC JSON_DATA' >&2
        return 2
    fi
    _daemon_post publish "{\"topic\":\"$1\",\"data\":$2}"
}
