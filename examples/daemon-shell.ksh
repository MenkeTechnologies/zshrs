# zshrs-daemon ksh wrappers (ksh93)
# =================================
# Source from your ~/.kshrc:
#
#     . /path/to/daemon-shell.ksh
#     daemon_ping
#     daemon_record_alias gst "git status"
#     daemon_defs_query --kind alias --shell-id ksh
#
# Targets ksh93u+m (the actively maintained AT&T fork) and pdksh /
# mksh. Function names use SNAKE_CASE because ksh93u+m rejects
# hyphens in identifiers entirely (`invalid function name: foo-bar`)
# regardless of `function NAME { … }` vs POSIX `name() { … }`. The
# bash/zsh/fish wrappers keep the hyphen style for typing ergonomics.
#
# WARNING: macOS's stock /bin/ksh is an old AT&T build that silently
# fails on multi-line awk inside function bodies AND on hyphenated
# function names. Use ksh93u+m (`brew install ksh93`), mksh, or
# daemon-shell.bash on macOS.
#
# Setup ($HOME/.zshrs/daemon.toml):
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
    typeset body="${1:-}"; [[ -z "$body" ]] && body='{}'
    _daemon_curl \
        -H 'Content-Type: application/json' \
        --data-raw "$body" \
        "$DAEMON_URL/op/$op"
}

function _daemon_get {
    _daemon_curl "$DAEMON_URL$1"
}

function _json_str {
    # ksh93's parser chokes on multi-line single-quoted awk inside a
    # function body — collapse the awk script to one line so this
    # sources cleanly under ksh93u+m as well as mksh / pdksh.
    printf '%s\1' "$1" | awk 'BEGIN{RS="\1"}{gsub(/\\/,"\\\\");gsub(/"/,"\\\"");gsub(/\t/,"\\t");gsub(/\r/,"\\r");gsub(/\n/,"\\n");printf"%s",$0;exit}'
}

# ---- Public commands ------------------------------------------------------

function daemon_health { _daemon_get /health; }
function daemon_ops    { _daemon_get /ops;    }
function daemon_info   { _daemon_post info '{}'; }

function daemon_ping {
    typeset payload='{}'
    [[ $# -gt 0 ]] && payload="{\"echo\":\"$*\"}"
    _daemon_post ping "$payload"
}

function daemon_call {
    typeset op="$1"; shift
    typeset body="${1:-}"; [[ -z "$body" ]] && body='{}'
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

function daemon_record_alias    { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_alias NAME BODY' >&2; return 2; }; _daemon_emit alias "$1" "$2"; }
function daemon_record_galias   { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_galias NAME BODY' >&2; return 2; }; _daemon_emit galias "$1" "$2"; }
function daemon_record_salias   { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_salias NAME BODY' >&2; return 2; }; _daemon_emit salias "$1" "$2"; }
function daemon_record_function { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_function NAME BODY' >&2; return 2; }; _daemon_emit function "$1" "$2"; }
function daemon_record_export   { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_export NAME VALUE' >&2; return 2; }; _daemon_emit env "$1" "$2"; }
function daemon_record_param    { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_param NAME VALUE' >&2; return 2; }; _daemon_emit params "$1" "$2"; }
function daemon_record_bindkey  { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_bindkey SEQ WIDGET' >&2; return 2; }; _daemon_emit bindkey "$1" "$2"; }
function daemon_record_compdef  { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_compdef CMD COMPLETER' >&2; return 2; }; _daemon_emit compdef "$1" "$2"; }
function daemon_record_zstyle   { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_zstyle PATTERN STYLE' >&2; return 2; }; _daemon_emit zstyle "$1" "$2"; }
function daemon_record_zmodload { [[ $# -lt 1 ]] && { echo 'usage: daemon_record_zmodload MODULE' >&2; return 2; }; _daemon_emit zmodload "$1"; }
function daemon_record_setopt   { [[ $# -lt 1 ]] && { echo 'usage: daemon_record_setopt OPT' >&2; return 2; }; _daemon_emit setopt "$1" on; }
function daemon_record_unsetopt { [[ $# -lt 1 ]] && { echo 'usage: daemon_record_unsetopt OPT' >&2; return 2; }; _daemon_emit setopt "$1" off; }
function daemon_record_source   { [[ $# -lt 1 ]] && { echo 'usage: daemon_record_source PATH' >&2; return 2; }; _daemon_emit source "$1"; }
function daemon_record_path     { [[ $# -lt 1 ]] && { echo 'usage: daemon_record_path DIR' >&2; return 2; }; _daemon_emit path "$1"; }
function daemon_record_fpath    { [[ $# -lt 1 ]] && { echo 'usage: daemon_record_fpath DIR' >&2; return 2; }; _daemon_emit fpath "$1"; }
function daemon_record_zle      { [[ $# -lt 1 ]] && { echo 'usage: daemon_record_zle WIDGET [BODY]' >&2; return 2; }; _daemon_emit zle "$1" "${2:-}"; }
function daemon_record_trap     { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_trap SIGNAL HANDLER' >&2; return 2; }; _daemon_emit trap "$1" "$2"; }
function daemon_record_named_dir { [[ $# -lt 2 ]] && { echo 'usage: daemon_record_named_dir NAME PATH' >&2; return 2; }; _daemon_emit named_dir "$1" "$2"; }
function daemon_record_completion { [[ $# -lt 1 ]] && { echo 'usage: daemon_record_completion CMD [PATH]' >&2; return 2; }; _daemon_emit completion "$1" "${2:-}"; }

# ---- Federated catalog query / diff --------------------------------------

function daemon_defs_query {
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

function daemon_defs_kinds { _daemon_post definitions_kinds '{}'; }

function daemon_defs_diff {
    [[ $# -lt 2 ]] && { echo 'usage: daemon_defs_diff SHELL_A SHELL_B [KIND]' >&2; return 2; }
    typeset body="{\"shell_a\":\"$1\",\"shell_b\":\"$2\""
    [[ -n "${3:-}" ]] && body="$body,\"kind\":\"$3\""
    body="$body}"
    _daemon_post definitions_diff "$body"
}

# ---- Streaming -----------------------------------------------------------

function daemon_watch {
    [[ $# -lt 1 ]] && { echo 'usage: daemon_watch DIR [--recursive]' >&2; return 2; }
    typeset dir="$1" recursive=false
    [[ "${2:-}" == '--recursive' ]] && recursive=true
    _daemon_curl -N "$DAEMON_URL/stream/watch?path=$dir&recursive=$recursive"
}

function daemon_events {
    typeset pat="${1:-*.*}"
    _daemon_curl -N "$DAEMON_URL/stream/events?channel=$pat"
}

function daemon_publish {
    [[ $# -ne 2 ]] && { echo 'usage: daemon_publish TOPIC JSON_DATA' >&2; return 2; }
    _daemon_post publish "{\"topic\":\"$1\",\"data\":$2}"
}
