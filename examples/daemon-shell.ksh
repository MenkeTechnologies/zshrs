# zshrs-daemon ksh wrappers (ksh93)
# =================================
# Source from your ~/.kshrc:
#
#     . /path/to/daemon-shell.ksh
#     daemon-ping
#     daemon-record-alias gst "git status"
#     daemon-defs-query --kind alias --shell-id ksh
#
# Targets ksh93 (modern AT&T ksh) and pdksh-derived shells with the
# `function NAME { … }` form (pdksh accepts both, ksh93 needs `function`
# for hyphenated names). Tested patterns also work in mksh.
#
# WARNING: macOS's stock /bin/ksh is an old AT&T build that silently
# fails on certain function definitions. Use a real ksh93u+m, mksh,
# or daemon-shell.bash on macOS.
#
# Setup ($HOME/.config/zshrs/daemon.toml):
#     [http]
#     listen = "127.0.0.1:7733"
#     # tokens = ["..."]
#
# Env overrides:
#     export DAEMON_URL=http://127.0.0.1:7733
#     export DAEMON_TOKEN=long-random-secret
#     export DAEMON_SHELL_ID=ksh         # see docs/SHELL_IDS.md

: ${DAEMON_URL:=http://127.0.0.1:7733}
: ${DAEMON_TOKEN:=}
: ${DAEMON_SHELL_ID:=ksh}

function _daemon_curl {
    if [[ -n "$DAEMON_TOKEN" ]]; then
        curl -sS -f -H "Authorization: Bearer $DAEMON_TOKEN" "$@"
    else
        curl -sS -f "$@"
    fi
}

function _daemon_post {
    typeset op="$1"; shift
    typeset body="${1:-{\}}"
    _daemon_curl \
        -H 'Content-Type: application/json' \
        --data-raw "$body" \
        "$DAEMON_URL/op/$op"
}

function _daemon_get {
    _daemon_curl "$DAEMON_URL$1"
}

function _json_str {
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

function daemon-health { _daemon_get /health; }
function daemon-ops    { _daemon_get /ops;    }
function daemon-info   { _daemon_post info '{}'; }

function daemon-ping {
    typeset payload='{}'
    [[ $# -gt 0 ]] && payload="{\"echo\":\"$*\"}"
    _daemon_post ping "$payload"
}

function daemon-call {
    typeset op="$1"; shift
    typeset body="${1:-{\}}"
    _daemon_post "$op" "$body"
}

# ---- Federated recorder (definitions.*) ----------------------------------

function _daemon_emit {
    typeset kind="$1" name="$2" value="${3:-}" file="${4:-}" line="${5:-}" chain="${6:-}"
    typeset body="{\"shell_id\":\"$DAEMON_SHELL_ID\",\"kind\":\"$kind\""
    body="$body,\"name\":\"$(_json_str "$name")\""
    [[ -n "$value" ]] && body="$body,\"value\":\"$(_json_str "$value")\""
    [[ -n "$file" ]]  && body="$body,\"file\":\"$(_json_str "$file")\""
    [[ -n "$line" ]]  && body="$body,\"line\":$line"
    [[ -n "$chain" ]] && body="$body,\"fn_chain\":\"$(_json_str "$chain")\""
    body="$body}"
    _daemon_post definitions_emit "$body"
}

function daemon-record-alias    { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-alias NAME BODY' >&2; return 2; }; _daemon_emit alias "$1" "$2"; }
function daemon-record-galias   { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-galias NAME BODY' >&2; return 2; }; _daemon_emit galias "$1" "$2"; }
function daemon-record-salias   { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-salias NAME BODY' >&2; return 2; }; _daemon_emit salias "$1" "$2"; }
function daemon-record-function { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-function NAME BODY' >&2; return 2; }; _daemon_emit function "$1" "$2"; }
function daemon-record-export   { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-export NAME VALUE' >&2; return 2; }; _daemon_emit env "$1" "$2"; }
function daemon-record-param    { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-param NAME VALUE' >&2; return 2; }; _daemon_emit params "$1" "$2"; }
function daemon-record-bindkey  { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-bindkey SEQ WIDGET' >&2; return 2; }; _daemon_emit bindkey "$1" "$2"; }
function daemon-record-compdef  { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-compdef CMD COMPLETER' >&2; return 2; }; _daemon_emit compdef "$1" "$2"; }
function daemon-record-zstyle   { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-zstyle PATTERN STYLE' >&2; return 2; }; _daemon_emit zstyle "$1" "$2"; }
function daemon-record-zmodload { [[ $# -lt 1 ]] && { echo 'usage: daemon-record-zmodload MODULE' >&2; return 2; }; _daemon_emit zmodload "$1"; }
function daemon-record-setopt   { [[ $# -lt 1 ]] && { echo 'usage: daemon-record-setopt OPT' >&2; return 2; }; _daemon_emit setopt "$1" on; }
function daemon-record-unsetopt { [[ $# -lt 1 ]] && { echo 'usage: daemon-record-unsetopt OPT' >&2; return 2; }; _daemon_emit setopt "$1" off; }
function daemon-record-source   { [[ $# -lt 1 ]] && { echo 'usage: daemon-record-source PATH' >&2; return 2; }; _daemon_emit source "$1"; }
function daemon-record-path     { [[ $# -lt 1 ]] && { echo 'usage: daemon-record-path DIR' >&2; return 2; }; _daemon_emit path "$1"; }
function daemon-record-fpath    { [[ $# -lt 1 ]] && { echo 'usage: daemon-record-fpath DIR' >&2; return 2; }; _daemon_emit fpath "$1"; }
function daemon-record-zle      { [[ $# -lt 1 ]] && { echo 'usage: daemon-record-zle WIDGET [BODY]' >&2; return 2; }; _daemon_emit zle "$1" "${2:-}"; }
function daemon-record-trap     { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-trap SIGNAL HANDLER' >&2; return 2; }; _daemon_emit trap "$1" "$2"; }
function daemon-record-named-dir { [[ $# -lt 2 ]] && { echo 'usage: daemon-record-named-dir NAME PATH' >&2; return 2; }; _daemon_emit named_dir "$1" "$2"; }
function daemon-record-completion { [[ $# -lt 1 ]] && { echo 'usage: daemon-record-completion CMD [PATH]' >&2; return 2; }; _daemon_emit completion "$1" "${2:-}"; }

# ---- Federated catalog query / diff --------------------------------------

function daemon-defs-query {
    typeset kind name prefix shell limit
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --kind)     kind="$2";   shift 2 ;;
            --name)     name="$2";   shift 2 ;;
            --prefix)   prefix="$2"; shift 2 ;;
            --shell-id) shell="$2";  shift 2 ;;
            --limit)    limit="$2";  shift 2 ;;
            *) echo "unknown arg: $1" >&2; return 2 ;;
        esac
    done
    typeset body='{' sep=
    [[ -n "$kind" ]]   && { body="$body${sep}\"kind\":\"$kind\"";       sep=','; }
    [[ -n "$name" ]]   && { body="$body${sep}\"name\":\"$name\"";       sep=','; }
    [[ -n "$prefix" ]] && { body="$body${sep}\"prefix\":\"$prefix\"";   sep=','; }
    [[ -n "$shell" ]]  && { body="$body${sep}\"shell_id\":\"$shell\""; sep=','; }
    [[ -n "$limit" ]]  && { body="$body${sep}\"limit\":$limit";         sep=','; }
    body="$body}"
    _daemon_post definitions_query "$body"
}

function daemon-defs-kinds { _daemon_post definitions_kinds '{}'; }

function daemon-defs-diff {
    [[ $# -lt 2 ]] && { echo 'usage: daemon-defs-diff SHELL_A SHELL_B [KIND]' >&2; return 2; }
    typeset body="{\"shell_a\":\"$1\",\"shell_b\":\"$2\""
    [[ -n "${3:-}" ]] && body="$body,\"kind\":\"$3\""
    body="$body}"
    _daemon_post definitions_diff "$body"
}

# ---- Streaming -----------------------------------------------------------

function daemon-watch {
    [[ $# -lt 1 ]] && { echo 'usage: daemon-watch DIR [--recursive]' >&2; return 2; }
    typeset dir="$1" recursive=false
    [[ "${2:-}" == '--recursive' ]] && recursive=true
    _daemon_curl -N "$DAEMON_URL/stream/watch?path=$dir&recursive=$recursive"
}

function daemon-events {
    typeset pat="${1:-*.*}"
    _daemon_curl -N "$DAEMON_URL/stream/events?channel=$pat"
}

function daemon-publish {
    [[ $# -ne 2 ]] && { echo 'usage: daemon-publish TOPIC JSON_DATA' >&2; return 2; }
    _daemon_post publish "{\"topic\":\"$1\",\"data\":$2}"
}
