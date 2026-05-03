# zshrs-daemon shell wrappers
# ===========================
# Source this file from any shell (bash/zsh/fish via `bass`/etc.) to
# get a clean CLI over the daemon's HTTP listener:
#
#     source /path/to/daemon-shell.zsh
#     daemon-ping
#     id=$(daemon-job-submit -- find / -name '*.zsh' 2>/dev/null)
#     daemon-job-status "$id"
#     daemon-job-output "$id"
#
# Setup ($HOME/.config/zshrs/daemon.toml):
#     [http]
#     listen = "127.0.0.1:7733"
#     # Optional — required if listening on a non-loopback interface:
#     # [http.tokens]
#     # mybox = "long-random-secret"
#
# Override the default base URL or token via env:
#     export DAEMON_URL=http://127.0.0.1:7733
#     export DAEMON_TOKEN=long-random-secret    # only if tokens configured
#
# All wrappers route through `curl -s -f --json` so JSON encoding,
# Authorization, and error propagation are handled once. Output is the
# daemon's raw JSON; pipe through `jq` for pretty-printing.

: ${DAEMON_URL:=http://127.0.0.1:7733}
: ${DAEMON_TOKEN:=}

# Internal: emit `Authorization: Bearer ...` only when DAEMON_TOKEN set.
# Echos a curl-`-H`-friendly arg list (one per line) so callers splice via
# `$(_daemon_auth_args)` without quoting hazards on empty token case.
_daemon_auth_args() {
    [[ -n "$DAEMON_TOKEN" ]] && printf -- '-H\nAuthorization: Bearer %s\n' "$DAEMON_TOKEN"
}

# Internal: POST to /op/<name> with JSON body. First arg = op name.
# Remaining args interpreted as raw JSON object body (single string).
_daemon_post() {
    local op="$1"; shift
    local body="${1:-{\}}"
    # shellcheck disable=SC2046
    curl -sS -f \
        -H 'Content-Type: application/json' \
        $(_daemon_auth_args) \
        --data-raw "$body" \
        "$DAEMON_URL/op/$op"
}

# Internal: GET an endpoint (/health, /ops).
# NOTE: do not name the local `path` — zsh ties `$path` to the
# `$PATH` array (`typeset -T PATH path :`), so `local path=...` shadows
# the command-search PATH and the very next `curl` call fails with
# "command not found: curl". Use `endpoint` instead.
_daemon_get() {
    local endpoint="$1"
    # shellcheck disable=SC2046
    curl -sS -f $(_daemon_auth_args) "$DAEMON_URL$endpoint"
}

# ---- Public commands ------------------------------------------------------

# `daemon-health` — liveness + version probe. Always unauthenticated.
daemon-health() { _daemon_get /health; }

# `daemon-ops` — list every op the daemon accepts.
daemon-ops() { _daemon_get /ops; }

# `daemon-ping` — round-trip latency + pong. Echoes any args back via the
# `echo` field so you can correlate logs.
daemon-ping() {
    local payload='{}'
    [[ $# -gt 0 ]] && payload="{\"echo\": \"$*\"}"
    _daemon_post ping "$payload"
}

# `daemon-info` — full daemon snapshot (catalog stats, uptime, paths).
daemon-info() { _daemon_post info '{}'; }

# `daemon-call OP [JSON]` — generic op caller. Use for ops that don't have
# a dedicated wrapper. Body defaults to {}.
#     daemon-call config_get '{"key":"long_cmd_threshold"}'
daemon-call() {
    local op="$1"; shift
    local body="${1:-{\}}"
    _daemon_post "$op" "$body"
}

# ---- Job runner -----------------------------------------------------------

# `daemon-job-submit [-c CWD] [-t TAG ...] -- COMMAND [ARGS ...]`
# Submits a long-running command to the daemon. Returns the job_id on
# stdout (one line). Use the id with daemon-job-{status,output,wait,kill}.
#
# Example (find every shell script under /, kill after 30s):
#     id=$(daemon-job-submit -- find / -type f -name '*.sh' 2>/dev/null)
#     daemon-job-wait "$id"             # blocks until done
#     daemon-job-output "$id" > shells.txt
daemon-job-submit() {
    local cwd=''
    local -a tags
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -c|--cwd)   shift; cwd="$1"; shift ;;
            -t|--tag)   shift; tags+=("$1"); shift ;;
            --)         shift; break ;;
            -*)         echo "daemon-job-submit: unknown flag: $1" >&2; return 2 ;;
            *)          break ;;
        esac
    done
    if [[ $# -eq 0 ]]; then
        echo 'usage: daemon-job-submit [-c CWD] [-t TAG ...] -- COMMAND [ARGS ...]' >&2
        return 2
    fi

    # Build JSON {command: [...], cwd?: "...", tags?: [...]}.
    # No jq dependency — emit raw JSON with shell quoting via printf %q
    # then escape for JSON. For full safety pipe through python -m json.tool
    # if the args contain wild characters; the simple path covers normal use.
    local args_json=''
    local first=1
    for a in "$@"; do
        local esc="${a//\\/\\\\}"; esc="${esc//\"/\\\"}"
        if (( first )); then
            args_json="\"$esc\""; first=0
        else
            args_json+=",\"$esc\""
        fi
    done
    local tags_json=''
    if (( ${#tags[@]} > 0 )); then
        first=1
        for t in "${tags[@]}"; do
            local esc="${t//\\/\\\\}"; esc="${esc//\"/\\\"}"
            if (( first )); then
                tags_json+="\"$esc\""; first=0
            else
                tags_json+=",\"$esc\""
            fi
        done
        tags_json=", \"tags\": [$tags_json]"
    fi
    local cwd_json=''
    [[ -n "$cwd" ]] && cwd_json=", \"cwd\": \"${cwd//\"/\\\"}\""

    local body="{\"command\": [$args_json]$cwd_json$tags_json}"
    # job_submit returns {"ok":true,"job_id":N}; pull just the id.
    _daemon_post job_submit "$body" \
        | sed -n 's/.*"job_id"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p'
}

# `daemon-job-status JOB_ID` — full status + exit code if finished.
daemon-job-status() {
    [[ $# -ne 1 ]] && { echo 'usage: daemon-job-status JOB_ID' >&2; return 2; }
    _daemon_post job_status "{\"id\": $1}"
}

# `daemon-job-output JOB_ID` — captured stdout/stderr (interleaved).
daemon-job-output() {
    [[ $# -ne 1 ]] && { echo 'usage: daemon-job-output JOB_ID' >&2; return 2; }
    _daemon_post job_output "{\"id\": $1}"
}

# `daemon-job-list` — every job the daemon knows about.
daemon-job-list() { _daemon_post job_list '{}'; }

# `daemon-job-kill JOB_ID` — SIGTERM the running process.
daemon-job-kill() {
    [[ $# -ne 1 ]] && { echo 'usage: daemon-job-kill JOB_ID' >&2; return 2; }
    _daemon_post job_kill "{\"id\": $1}"
}

# `daemon-job-wait JOB_ID [POLL_SECS]` — block until job finishes. Polls
# job_status every POLL_SECS seconds (default 1). Echoes the final status
# JSON when done, returns the job's exit code.
daemon-job-wait() {
    [[ $# -lt 1 ]] && { echo 'usage: daemon-job-wait JOB_ID [POLL_SECS]' >&2; return 2; }
    local id="$1"
    local poll="${2:-1}"
    while :; do
        local resp
        resp=$(daemon-job-status "$id") || return 1
        # Pull `state` field. Terminal states: exited, failed, killed,
        # cancelled. Pre-terminal: pending, running.
        local st
        st=$(printf '%s' "$resp" | sed -n 's/.*"state"[[:space:]]*:[[:space:]]*"\([a-z]*\)".*/\1/p')
        case "$st" in
            exited|failed|killed|cancelled)
                printf '%s\n' "$resp"
                # Pull exit_code; non-zero / missing → return 1.
                local code
                code=$(printf '%s' "$resp" | sed -n 's/.*"exit_code"[[:space:]]*:[[:space:]]*\([0-9-][0-9]*\).*/\1/p')
                if [[ "$st" == "exited" && "$code" == "0" ]]; then return 0; fi
                return 1
                ;;
            *)
                sleep "$poll"
                ;;
        esac
    done
}

# ---- Export wrappers ------------------------------------------------------

# `daemon-export TARGET [FORMAT]` — dump a canonical subsystem in the
# requested format. TARGET = alias|function|env|path|fpath|... ;
# FORMAT = sh (default) | json | yaml | text | pdf.
#     daemon-export alias json | jq .
#     daemon-export function | tee my-funcs.sh
daemon-export() {
    [[ $# -lt 1 ]] && { echo 'usage: daemon-export TARGET [FORMAT]' >&2; return 2; }
    local target="$1"
    local format="${2:-sh}"
    _daemon_post export "{\"target\": \"$target\", \"format\": \"$format\"}"
}

# `daemon-export-pdf TARGET OUTFILE` — convenience: hit the export op
# with format=pdf and write the decoded PDF bytes to OUTFILE.
#     daemon-export-pdf alias my-aliases.pdf
daemon-export-pdf() {
    [[ $# -ne 2 ]] && { echo 'usage: daemon-export-pdf TARGET OUTFILE' >&2; return 2; }
    local target="$1"
    local out="$2"
    local resp
    resp=$(_daemon_post export "{\"target\": \"$target\", \"format\": \"pdf\"}") || return 1
    # Response shape: {"ok":true,"format":"pdf","body_base64":"..."}.
    printf '%s' "$resp" \
        | sed -n 's/.*"body_base64"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        | base64 -d > "$out"
    [[ -s "$out" ]] || { echo "daemon-export-pdf: empty PDF — check target/format" >&2; return 1; }
    echo "wrote $out ($(wc -c < "$out") bytes)"
}

# ---- KV cache (daemon.cache.*) -------------------------------------------

_json_str() {
    # Minimal JSON-string escape for the `value` field. Handles backslash,
    # double-quote, newline, tab, CR, formfeed, backspace. Anything else
    # (control chars 0x00-0x1F not in that set) is passed through; for
    # binary data, base64-encode and use a different op (the daemon
    # accepts raw strings only on cache_put).
    printf '%s' "$1" \
        | sed -e 's/\\/\\\\/g' \
              -e 's/"/\\"/g' \
              -e 's/\t/\\t/g' \
              -e 's/\r/\\r/g' \
        | tr '\n' '\f' | sed 's/\f/\\n/g'
}

# `daemon-cache-put NS KEY VALUE [TTL_SECS]` — store a key/value.
daemon-cache-put() {
    [[ $# -lt 3 ]] && { echo 'usage: daemon-cache-put NS KEY VALUE [TTL_SECS]' >&2; return 2; }
    local body="{\"ns\":\"$1\",\"key\":\"$2\",\"value\":\"$(_json_str "$3")\""
    [[ -n "${4:-}" ]] && body+=", \"ttl_secs\": $4"
    body+="}"
    _daemon_post cache_put "$body"
}

# `daemon-cache-get NS KEY` — fetch raw value (the `value` JSON field).
daemon-cache-get() {
    [[ $# -ne 2 ]] && { echo 'usage: daemon-cache-get NS KEY' >&2; return 2; }
    _daemon_post cache_get "{\"ns\":\"$1\",\"key\":\"$2\"}" \
        | sed -n 's/.*"value"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
}

# `daemon-cache-del NS KEY` — remove a key.
daemon-cache-del() {
    [[ $# -ne 2 ]] && { echo 'usage: daemon-cache-del NS KEY' >&2; return 2; }
    _daemon_post cache_del "{\"ns\":\"$1\",\"key\":\"$2\"}"
}

# `daemon-cache-list NS [PREFIX]` — list keys.
daemon-cache-list() {
    [[ $# -lt 1 ]] && { echo 'usage: daemon-cache-list NS [PREFIX]' >&2; return 2; }
    local body="{\"ns\":\"$1\""
    [[ -n "${2:-}" ]] && body+=", \"prefix\": \"$2\""
    body+="}"
    _daemon_post cache_list "$body"
}

# `daemon-cache-stats [NS]` — byte/key counts.
daemon-cache-stats() {
    if [[ $# -eq 0 ]]; then
        _daemon_post cache_stats '{}'
    else
        _daemon_post cache_stats "{\"ns\":\"$1\"}"
    fi
}

# ---- Locks (daemon.lock.*) -----------------------------------------------

# `daemon-lock-acquire NAME [TIMEOUT_SECS]` — block until acquired
# (or until TIMEOUT_SECS elapses). Echos the token on success.
daemon-lock-acquire() {
    [[ $# -lt 1 ]] && { echo 'usage: daemon-lock-acquire NAME [TIMEOUT_SECS]' >&2; return 2; }
    local body="{\"name\":\"$1\",\"pid\":$$"
    [[ -n "${2:-}" ]] && body+=", \"timeout_secs\": $2"
    body+="}"
    _daemon_post lock_acquire "$body" \
        | sed -n 's/.*"token"[[:space:]]*:[[:space:]]*"\([0-9]*\)".*/\1/p'
}

# `daemon-lock-try NAME` — non-blocking acquire; echos token or `busy`.
daemon-lock-try() {
    [[ $# -ne 1 ]] && { echo 'usage: daemon-lock-try NAME' >&2; return 2; }
    _daemon_post lock_try_acquire "{\"name\":\"$1\",\"pid\":$$}"
}

# `daemon-lock-release NAME TOKEN` — release a previously acquired lock.
daemon-lock-release() {
    [[ $# -ne 2 ]] && { echo 'usage: daemon-lock-release NAME TOKEN' >&2; return 2; }
    _daemon_post lock_release "{\"name\":\"$1\",\"token\":\"$2\"}"
}

# `daemon-lock-list` — list active locks.
daemon-lock-list() { _daemon_post lock_list '{}'; }

# `daemon-lock-do NAME -- COMMAND ...` — run COMMAND under a lock,
# automatically releasing on exit. Bails if lock not available
# (no timeout; use daemon-lock-acquire then trap if you want one).
#     daemon-lock-do prod-deploy -- ./deploy.sh prod
daemon-lock-do() {
    local name="$1"
    [[ "$2" != "--" ]] && { echo 'usage: daemon-lock-do NAME -- CMD ...' >&2; return 2; }
    shift 2
    local resp
    resp=$(daemon-lock-try "$name") || return 1
    local tok
    tok=$(printf '%s' "$resp" | sed -n 's/.*"token"[[:space:]]*:[[:space:]]*"\([0-9]*\)".*/\1/p')
    [[ -z "$tok" ]] && { echo "daemon-lock-do: lock `$name` is held: $resp" >&2; return 1; }
    trap 'daemon-lock-release '"$name"' '"$tok"' >/dev/null' EXIT INT TERM
    "$@"
    local rc=$?
    daemon-lock-release "$name" "$tok" >/dev/null
    trap - EXIT INT TERM
    return $rc
}

# ---- Snapshots (daemon.snapshot.*) ---------------------------------------

# `daemon-snapshot-save TAG` — capture canonical state under TAG.
daemon-snapshot-save() {
    [[ $# -ne 1 ]] && { echo 'usage: daemon-snapshot-save TAG' >&2; return 2; }
    _daemon_post snapshot_save "{\"tag\":\"$1\"}"
}

# `daemon-snapshot-list` — every saved snapshot tag.
daemon-snapshot-list() { _daemon_post snapshot_list '{}'; }

# `daemon-snapshot-load TAG` — restore canonical state from TAG (atomic).
daemon-snapshot-load() {
    [[ $# -ne 1 ]] && { echo 'usage: daemon-snapshot-load TAG' >&2; return 2; }
    _daemon_post snapshot_load "{\"tag\":\"$1\"}"
}

# `daemon-snapshot-diff A B` — added/removed/changed between two tags.
daemon-snapshot-diff() {
    [[ $# -ne 2 ]] && { echo 'usage: daemon-snapshot-diff A B' >&2; return 2; }
    _daemon_post snapshot_diff "{\"a\":\"$1\",\"b\":\"$2\"}"
}

# ---- Artifact cache (daemon.artifact.*) ----------------------------------

# `daemon-artifact-put NAME FILE` — store FILE bytes under NAME (sha256
# computed by the daemon). Returns the digest.
daemon-artifact-put() {
    [[ $# -ne 2 ]] && { echo 'usage: daemon-artifact-put NAME FILE' >&2; return 2; }
    local name="$1"
    local file="$2"
    [[ -r "$file" ]] || { echo "daemon-artifact-put: cannot read $file" >&2; return 1; }
    local b64
    b64=$(base64 < "$file" | tr -d '\n')
    _daemon_post artifact_put "{\"name\":\"$name\",\"value_base64\":\"$b64\"}"
}

# `daemon-artifact-get NAME OUTFILE` — fetch artifact by name into OUTFILE.
daemon-artifact-get() {
    [[ $# -ne 2 ]] && { echo 'usage: daemon-artifact-get NAME OUTFILE' >&2; return 2; }
    local name="$1"
    local out="$2"
    _daemon_post artifact_get "{\"name\":\"$name\"}" \
        | sed -n 's/.*"value_base64"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        | base64 -d > "$out"
    [[ -s "$out" ]] || { echo "daemon-artifact-get: artifact $name empty/missing" >&2; return 1; }
    echo "wrote $out ($(wc -c < "$out") bytes)"
}

# `daemon-artifact-list [PREFIX]` — list artifacts.
daemon-artifact-list() {
    if [[ $# -eq 0 ]]; then
        _daemon_post artifact_list '{}'
    else
        _daemon_post artifact_list "{\"prefix\":\"$1\"}"
    fi
}

# `daemon-artifact-gc [MAX_AGE_SECS] [MAX_BYTES]` — collect old/oversized.
daemon-artifact-gc() {
    local body='{}'
    if [[ $# -eq 1 ]]; then
        body="{\"max_age_secs\": $1}"
    elif [[ $# -ge 2 ]]; then
        body="{\"max_age_secs\": $1, \"max_bytes\": $2}"
    fi
    _daemon_post artifact_gc "$body"
}

# ---- Schedule (daemon.schedule.*) ----------------------------------------

# `daemon-schedule-add CRON_EXPR -- COMMAND ...` — register a recurring job.
# CRON_EXPR is a 6-field spec: "sec min hr dom mon dow". Examples:
#   daemon-schedule-add '*/2 * * * * *'        -- echo tick
#   daemon-schedule-add '0 0 3 * * *'          -- backup-photos.sh
#   daemon-schedule-add '0 */15 * * * *'       -- check-mail.sh
daemon-schedule-add() {
    [[ $# -lt 3 || "$2" != "--" ]] && {
        echo 'usage: daemon-schedule-add CRON_EXPR -- COMMAND ...' >&2; return 2
    }
    local cron="$1"; shift 2
    local cmd_json='' first=1
    for a in "$@"; do
        local esc="${a//\\/\\\\}"; esc="${esc//\"/\\\"}"
        if (( first )); then cmd_json+="\"$esc\""; first=0
        else cmd_json+=",\"$esc\""; fi
    done
    _daemon_post schedule_add "{\"cron_expr\":\"$cron\",\"command\":[$cmd_json]}"
}

# `daemon-schedule-add-once UNIX_SECS -- COMMAND ...` — one-shot fire.
daemon-schedule-add-once() {
    [[ $# -lt 3 || "$2" != "--" ]] && {
        echo 'usage: daemon-schedule-add-once UNIX_SECS -- COMMAND ...' >&2; return 2
    }
    local fire="$1"; shift 2
    local cmd_json='' first=1
    for a in "$@"; do
        local esc="${a//\\/\\\\}"; esc="${esc//\"/\\\"}"
        if (( first )); then cmd_json+="\"$esc\""; first=0
        else cmd_json+=",\"$esc\""; fi
    done
    _daemon_post schedule_add_once "{\"fire_at_unix_secs\":$fire,\"command\":[$cmd_json]}"
}

# `daemon-schedule-list` / `daemon-schedule-remove ID`.
daemon-schedule-list()   { _daemon_post schedule_list '{}'; }
daemon-schedule-remove() {
    [[ $# -ne 1 ]] && { echo 'usage: daemon-schedule-remove ID' >&2; return 2; }
    _daemon_post schedule_remove "{\"id\":$1}"
}

# ---- SSE streams ---------------------------------------------------------

# `daemon-watch DIR [--recursive]` — stream filesystem events for DIR
# as Server-Sent Events. Each line is a JSON record with event=fs.
# Pipe through `sed -n '/^data:/s/^data: //p'` to extract payloads.
#     daemon-watch ~/src | sed -n '/^data:/s/^data: //p' | jq -r .trigger_path
daemon-watch() {
    [[ $# -lt 1 ]] && { echo 'usage: daemon-watch DIR [--recursive]' >&2; return 2; }
    local dir="$1"
    local recursive='false'
    [[ "${2:-}" == "--recursive" ]] && recursive='true'
    # shellcheck disable=SC2046
    curl -sN $(_daemon_auth_args) \
        "$DAEMON_URL/stream/watch?path=$dir&recursive=$recursive"
}

# `daemon-events [PATTERN]` — stream pubsub messages matching PATTERN
# (default `*.*` = everything). Pattern is `<scope>.<topic>`.
#     daemon-events 'shell:*.build_done' | sed -n '/^data:/s/^data: //p' | jq
daemon-events() {
    local pat="${1:-*.*}"
    # shellcheck disable=SC2046
    curl -sN $(_daemon_auth_args) \
        "$DAEMON_URL/stream/events?channel=$pat"
}

# `daemon-publish TOPIC JSON_DATA` — publish a pubsub event.
daemon-publish() {
    [[ $# -ne 2 ]] && { echo 'usage: daemon-publish TOPIC JSON_DATA' >&2; return 2; }
    local topic="$1"
    local data="$2"
    _daemon_post publish "{\"topic\":\"$topic\",\"data\":$data}"
}
