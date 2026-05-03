# daemon-shell.nu — nushell client for zshrs-daemon's HTTP service.
#
# Source from your config (`config nu` then add at bottom, or via
# $env.config.nu_files):
#
#     source path/to/daemon-shell.nu
#     daemon ping
#     daemon record alias gst "git status"
#     daemon defs query --kind alias --shell-id nu
#
# Nushell idiom: subcommands under `daemon …` (multi-word custom commands)
# rather than hyphen-prefixed names — feels native, completes naturally.
#
# Endpoint and auth come from env (in your config.nu):
#
#     $env.DAEMON_URL = "http://127.0.0.1:7733"
#     $env.DAEMON_TOKEN = "<bearer-token>"      # optional
#     $env.DAEMON_SHELL_ID = "nu"               # see docs/SHELL_IDS.md
#
# All commands return parsed records / tables — `http get` and
# `http post` decode JSON automatically. No `jq` needed.

# ---- Config defaults ------------------------------------------------------

if not ('DAEMON_URL'      in $env) { $env.DAEMON_URL      = "http://127.0.0.1:7733" }
if not ('DAEMON_TOKEN'    in $env) { $env.DAEMON_TOKEN    = "" }
if not ('DAEMON_SHELL_ID' in $env) { $env.DAEMON_SHELL_ID = "nu" }

# ---- Internal helpers -----------------------------------------------------

# Build the auth header record. nushell's `http` accepts `--headers`
# as a flat list `[name value name value]`.
def _daemon_headers []: nothing -> list<string> {
    if ($env.DAEMON_TOKEN | is-empty) {
        []
    } else {
        ["Authorization" $"Bearer ($env.DAEMON_TOKEN)"]
    }
}

# Internal: POST /op/<name> with JSON body record.
def _daemon_post [op: string, body: record = {}]: nothing -> any {
    http post --content-type application/json \
              --headers (_daemon_headers) \
              $"($env.DAEMON_URL)/op/($op)" \
              $body
}

# Internal: GET an endpoint (/health, /ops, /metrics).
def _daemon_get [endpoint: string]: nothing -> any {
    http get --headers (_daemon_headers) $"($env.DAEMON_URL)($endpoint)"
}

# ---- Public commands ------------------------------------------------------

# `daemon health` — liveness + version probe.
def "daemon health" []: nothing -> any { _daemon_get "/health" }

# `daemon ops` — list every op the daemon accepts.
def "daemon ops" []: nothing -> any { _daemon_get "/ops" }

# `daemon info` — full daemon snapshot.
def "daemon info" []: nothing -> any { _daemon_post info {} }

# `daemon ping [echo]` — round-trip latency.
def "daemon ping" [...echo: string]: nothing -> any {
    let body = if ($echo | is-empty) { {} } else { { echo: ($echo | str join " ") } }
    _daemon_post ping $body
}

# `daemon call OP [JSON-RECORD]` — generic op caller for ops without
# a dedicated wrapper.
def "daemon call" [op: string, body: record = {}]: nothing -> any {
    _daemon_post $op $body
}

# ---- Federated recorder (definitions.*) ----------------------------------

def _daemon_emit [
    kind: string,
    name: string,
    value: string = "",
    --file: string,
    --line: int,
    --fn-chain: string,
]: nothing -> any {
    mut body = {
        shell_id: $env.DAEMON_SHELL_ID
        kind: $kind
        name: $name
    }
    if (not ($value | is-empty)) { $body = ($body | upsert value $value) }
    if ($file? | is-not-empty)   { $body = ($body | upsert file $file) }
    if ($line? | is-not-empty)   { $body = ($body | upsert line $line) }
    if ($fn_chain? | is-not-empty) { $body = ($body | upsert fn_chain $fn_chain) }
    _daemon_post definitions_emit $body
}

def "daemon record alias"     [name: string, body: string]: nothing -> any { _daemon_emit alias $name $body }
def "daemon record galias"    [name: string, body: string]: nothing -> any { _daemon_emit galias $name $body }
def "daemon record salias"    [name: string, body: string]: nothing -> any { _daemon_emit salias $name $body }
def "daemon record function"  [name: string, body: string]: nothing -> any { _daemon_emit function $name $body }
def "daemon record export"    [name: string, value: string]: nothing -> any { _daemon_emit env $name $value }
def "daemon record param"     [name: string, value: string]: nothing -> any { _daemon_emit params $name $value }
def "daemon record bindkey"   [seq: string, widget: string]: nothing -> any { _daemon_emit bindkey $seq $widget }
def "daemon record compdef"   [cmd: string, completer: string]: nothing -> any { _daemon_emit compdef $cmd $completer }
def "daemon record zstyle"    [pattern: string, style: string]: nothing -> any { _daemon_emit zstyle $pattern $style }
def "daemon record zmodload"  [module: string]: nothing -> any { _daemon_emit zmodload $module }
def "daemon record setopt"    [opt: string]: nothing -> any { _daemon_emit setopt $opt "on" }
def "daemon record unsetopt"  [opt: string]: nothing -> any { _daemon_emit setopt $opt "off" }
def "daemon record source"    [path: string]: nothing -> any { _daemon_emit source $path }
def "daemon record path"      [dir: string]: nothing -> any { _daemon_emit path $dir }
def "daemon record fpath"     [dir: string]: nothing -> any { _daemon_emit fpath $dir }
def "daemon record zle"       [widget: string, body: string = ""]: nothing -> any { _daemon_emit zle $widget $body }
def "daemon record trap"      [signal: string, handler: string]: nothing -> any { _daemon_emit trap $signal $handler }
def "daemon record named-dir" [name: string, path: string]: nothing -> any { _daemon_emit named_dir $name $path }
def "daemon record completion" [cmd: string, path: string = ""]: nothing -> any { _daemon_emit completion $cmd $path }

# ---- Federated catalog query / diff --------------------------------------

# `daemon defs query [--kind K] [--name N] [--prefix P] [--shell-id S] [--limit N]`
def "daemon defs query" [
    --kind: string,
    --name: string,
    --prefix: string,
    --shell-id: string,
    --limit: int,
]: nothing -> any {
    mut body = {}
    if ($kind?     | is-not-empty) { $body = ($body | upsert kind $kind) }
    if ($name?     | is-not-empty) { $body = ($body | upsert name $name) }
    if ($prefix?   | is-not-empty) { $body = ($body | upsert prefix $prefix) }
    if ($shell_id? | is-not-empty) { $body = ($body | upsert shell_id $shell_id) }
    if ($limit?    | is-not-empty) { $body = ($body | upsert limit $limit) }
    _daemon_post definitions_query $body
}

def "daemon defs kinds" []: nothing -> any { _daemon_post definitions_kinds {} }

# `daemon defs diff SHELL_A SHELL_B [KIND]`
def "daemon defs diff" [shell_a: string, shell_b: string, kind?: string]: nothing -> any {
    mut body = { shell_a: $shell_a, shell_b: $shell_b }
    if ($kind | is-not-empty) { $body = ($body | upsert kind $kind) }
    _daemon_post definitions_diff $body
}

# ---- Pubsub --------------------------------------------------------------
#
# Streaming endpoints (/stream/*) deliver SSE; nushell's `http get` doesn't
# do streaming today, so use `curl -N` for `daemon watch` / `daemon events`
# from a regular $env.DAEMON_URL setup. `daemon publish` is a one-shot
# POST and works fine.

# `daemon publish TOPIC DATA-RECORD`
def "daemon publish" [topic: string, data: any]: nothing -> any {
    _daemon_post publish { topic: $topic, data: $data }
}
