# zshrs-daemon HTTP cookbook

Quick-reference recipes for hitting the daemon's HTTP listener with `curl`,
`httpie`, browser fetch, and other generic HTTP clients. Pairs with the
shell-function wrappers in [`daemon-shell.zsh`](./daemon-shell.zsh).

## Setup

`~/.zshrs/daemon.toml`:

```toml
[http]
listen = "127.0.0.1:7733"

# OPTIONAL — required if listen is a non-loopback address.
# Without tokens the daemon refuses to bind on non-127.0.0.1 (safety floor).
[http.tokens]
me     = "long-random-secret-1"
ci     = "long-random-secret-2"
vim-lsp = "long-random-secret-3"
```

Restart `zshrs-daemon` after editing.

## Endpoints

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/health` | none | liveness + version |
| `GET` | `/ops` | none | list every op name the daemon accepts |
| `POST` | `/op/<NAME>` | bearer if configured | invoke op `<NAME>` with JSON body |

Op JSON body shape matches the existing IPC `args` payload — same surface,
different transport.

## Auth header

When tokens are configured, every `POST /op/<NAME>` must carry:

```
Authorization: Bearer <token>
```

Missing/wrong token → `401 {"ok":false,"code":"unauthorized","msg":"..."}`.

## curl recipes

### Liveness

```sh
curl -s http://127.0.0.1:7733/health
# {"ok":true,"version":"0.12.0","uptime_ms":12345}
```

### Op list

```sh
curl -s http://127.0.0.1:7733/ops | jq '.ops[]'
```

### Ping

```sh
curl -s -X POST http://127.0.0.1:7733/op/ping \
     -H 'Content-Type: application/json' \
     -d '{}'
# {"ok":true,"pong":true,"ts_ns":...,"daemon_uptime_ms":...,"echo":null}
```

### Daemon info

```sh
curl -s -X POST http://127.0.0.1:7733/op/info \
     -H 'Content-Type: application/json' \
     -d '{}' | jq
```

### With bearer token

```sh
curl -s -X POST http://127.0.0.1:7733/op/info \
     -H 'Authorization: Bearer long-random-secret-1' \
     -H 'Content-Type: application/json' \
     -d '{}' | jq
```

## Submitting a long-running job

The daemon owns a tokio-backed job queue. Submit a command, get a job_id,
poll for completion, fetch output. Job processes survive client
disconnects (the daemon owns the child).

### `find / -name '*.zsh'` end-to-end

```sh
# 1. Submit. command must be a JSON array (argv-style).
JOB=$(curl -s -X POST http://127.0.0.1:7733/op/job_submit \
    -H 'Content-Type: application/json' \
    -d '{"command":["find","/","-type","f","-name","*.zsh"]}' \
  | jq -r .job_id)
echo "submitted job_id=$JOB"

# 2. Poll status (blocks via shell loop; daemon doesn't push).
# Terminal states: exited (clean exit, check exit_code), failed,
# killed, cancelled. Pre-terminal: pending, running.
while :; do
    ST=$(curl -s -X POST http://127.0.0.1:7733/op/job_status \
        -H 'Content-Type: application/json' \
        -d "{\"id\":$JOB}" | jq -r .job.state)
    echo "state=$ST"
    case "$ST" in
        exited|failed|killed|cancelled) break ;;
    esac
    sleep 1
done

# 3. Fetch captured stdout/stderr. The op returns the full file body
# in the `content` field. Pass `"stderr": true` to fetch the err
# stream instead of stdout.
curl -s -X POST http://127.0.0.1:7733/op/job_output \
    -H 'Content-Type: application/json' \
    -d "{\"id\":$JOB,\"stderr\":false}" | jq -r .content > shells.txt

wc -l shells.txt
```

### Job submission payload reference

```json
{
  "command": ["argv0", "arg1", "arg2"],   // required, non-empty
  "cwd":     "/path/to/workdir",          // optional; daemon's cwd by default
  "env":     {"KEY1": "val1"},            // optional; merged into daemon env
  "tags":    ["nightly", "deploy"]        // optional; for daemon.job_list filtering
}
```

### Listing / killing

```sh
# All jobs:
curl -s -X POST http://127.0.0.1:7733/op/job_list -d '{}'

# Filter by state:
curl -s -X POST http://127.0.0.1:7733/op/job_list -d '{"state":"running"}'

# Kill (SIGTERM):
curl -s -X POST http://127.0.0.1:7733/op/job_kill -d "{\"id\":$JOB}"
```

## Exporting daemon state

`op_export` dumps any canonical subsystem (alias, function, env, path,
fpath, manpath, named_dir, zstyle, bindkey, setopt, zmodload, compdef,
zle widget catalog, recorder-discovered completions, etc.) in the
requested format.

| `format` | Output |
|---|---|
| `sh` (default) | shell-source-able lines (`alias gst='git status -sb'`) |
| `json` | JSON object/array per subsystem |
| `yaml` | YAML — same shape as json |
| `text` | human-readable, no escaping |
| `pdf` | PDF document; `body_base64` field carries the bytes |

### Aliases as JSON

```sh
curl -s -X POST http://127.0.0.1:7733/op/export \
     -H 'Content-Type: application/json' \
     -d '{"target":"alias","format":"json"}' | jq
```

### Functions as shell source (eval-able)

```sh
curl -s -X POST http://127.0.0.1:7733/op/export \
     -H 'Content-Type: application/json' \
     -d '{"target":"function","format":"sh"}' > my-funcs.sh
source my-funcs.sh   # restores every captured function
```

### PDF

```sh
curl -s -X POST http://127.0.0.1:7733/op/export \
     -H 'Content-Type: application/json' \
     -d '{"target":"alias","format":"pdf"}' \
  | jq -r .body_base64 \
  | base64 -d > aliases.pdf

open aliases.pdf
```

PDF output is suitable for compliance / archival snapshots — same data
the JSON/SH dumps carry, formatted as a paginated document.

## httpie recipes

[httpie](https://httpie.io) is a friendlier curl. All examples translate
1:1; here are the most common:

```sh
http :7733/health
http :7733/ops

# POST with JSON body — httpie infers Content-Type
http POST :7733/op/ping
http POST :7733/op/info

http POST :7733/op/job_submit \
    command:='["find","/","-name","*.zsh"]'

http POST :7733/op/export \
    target=alias format=json | jq

# With auth:
http POST :7733/op/info \
    Authorization:'Bearer long-random-secret-1'
```

## Browser / fetch / other clients

The endpoints are plain HTTP/1.1 + JSON; any client works. Browser
console example:

```js
fetch('http://127.0.0.1:7733/op/ping', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: '{}',
}).then(r => r.json()).then(console.log)
```

Note CORS: the daemon does NOT set `Access-Control-Allow-Origin` headers
in v1 — designed for same-origin tooling (CLI, editor plugins, scripts),
not browser pages from the open web. Add a reverse proxy (nginx,
Caddy) if you need browser access.

## KV cache (`daemon.cache.*`)

Persistent namespaced KV, sqlite-backed at `~/.zshrs/cache.db`.
Replaces single-user Redis. Namespaces (`ns`) are arbitrary strings.

```sh
# Store a build config:
curl -s -X POST http://127.0.0.1:7733/op/cache_put \
    -H 'Content-Type: application/json' \
    -d '{"ns":"ci","key":"target","value":"release","ttl_secs":86400}'
# {"ok":true,"bytes":7,"key":"target","ns":"ci","expires_at":...}

# Read it back:
curl -s -X POST http://127.0.0.1:7733/op/cache_get \
    -H 'Content-Type: application/json' \
    -d '{"ns":"ci","key":"target"}' | jq -r .value
# release

# List keys in namespace:
curl -s -X POST http://127.0.0.1:7733/op/cache_list \
    -H 'Content-Type: application/json' \
    -d '{"ns":"ci"}'

# Stats per namespace OR globally:
curl -s -X POST http://127.0.0.1:7733/op/cache_stats \
    -H 'Content-Type: application/json' -d '{}'

# Delete:
curl -s -X POST http://127.0.0.1:7733/op/cache_del \
    -H 'Content-Type: application/json' \
    -d '{"ns":"ci","key":"target"}'
```

## Locks (`daemon.lock.*`)

Named cross-process mutexes with PID-tied auto-release. Replaces `flock(1)`.

```sh
# Try to acquire (non-blocking; pid lets daemon force-release on crash):
RESP=$(curl -s -X POST http://127.0.0.1:7733/op/lock_try_acquire \
    -H 'Content-Type: application/json' \
    -d "{\"name\":\"deploy\",\"pid\":$$}")
TOK=$(echo "$RESP" | jq -r .token)

# Block-acquire with timeout:
curl -s -X POST http://127.0.0.1:7733/op/lock_acquire \
    -H 'Content-Type: application/json' \
    -d "{\"name\":\"deploy\",\"pid\":$$,\"timeout_secs\":30}"

# Critical section here.
trap 'curl -s -X POST http://127.0.0.1:7733/op/lock_release \
    -H "Content-Type: application/json" \
    -d "{\"name\":\"deploy\",\"token\":\"'$TOK'\"}"' EXIT

# What's currently held:
curl -s -X POST http://127.0.0.1:7733/op/lock_list \
    -H 'Content-Type: application/json' -d '{}'
```

## Artifact cache (`daemon.artifact.*`)

Content-addressed via sha256. Multiple names can point at the same digest
(automatic dedup). Wire-encodes blob via base64.

```sh
# Store a binary artifact:
B64=$(base64 < ./output.o | tr -d '\n')
curl -s -X POST http://127.0.0.1:7733/op/artifact_put \
    -H 'Content-Type: application/json' \
    -d "{\"name\":\"compile-$(sha256sum input.c | cut -c1-16)\",\"value_base64\":\"$B64\"}"

# Fetch back into a file:
curl -s -X POST http://127.0.0.1:7733/op/artifact_get \
    -H 'Content-Type: application/json' \
    -d '{"name":"compile-abc123"}' \
  | jq -r .value_base64 | base64 -d > output.o

# Fetch by digest (without name):
curl -s -X POST http://127.0.0.1:7733/op/artifact_get_by_digest \
    -H 'Content-Type: application/json' \
    -d '{"digest":"4de3c2a92260a4efbe6dc487d02b113fed99d11244c2baf8bdbf7dea184cff75"}'

# List + GC (drop entries older than 30 days OR over 100MB total):
curl -s -X POST http://127.0.0.1:7733/op/artifact_list \
    -H 'Content-Type: application/json' -d '{}'

curl -s -X POST http://127.0.0.1:7733/op/artifact_gc \
    -H 'Content-Type: application/json' \
    -d '{"max_age_secs":2592000,"max_bytes":104857600}'
```

## Snapshots (`daemon.snapshot.*`)

Tag-based canonical-state capture/restore. Same on-disk format as the
recorder bundle — rkyv `CanonicalShard` files under
`~/.zshrs/snapshots/<tag>.rkyv`.

```sh
# Save state under a tag:
curl -s -X POST http://127.0.0.1:7733/op/snapshot_save \
    -H 'Content-Type: application/json' \
    -d '{"tag":"laptop-pre-experiment"}'

# List snapshots:
curl -s -X POST http://127.0.0.1:7733/op/snapshot_list \
    -H 'Content-Type: application/json' -d '{}'

# Diff two snapshots (added/removed/changed records):
curl -s -X POST http://127.0.0.1:7733/op/snapshot_diff \
    -H 'Content-Type: application/json' \
    -d '{"a":"baseline","b":"after-plugin-install"}'

# Restore (atomic swap of canonical state):
curl -s -X POST http://127.0.0.1:7733/op/snapshot_load \
    -H 'Content-Type: application/json' \
    -d '{"tag":"laptop-pre-experiment"}'
```

## Schedule (`daemon.schedule.*`)

Cron-equivalent recurring + one-shot jobs. State persists across daemon
restarts. The schedule tick runs every 1 second, dispatches `job_submit`
for each due row, and tags the resulting jobs with `"scheduled"`.

Cron format is **6 fields** including seconds: `sec min hr dom mon dow`.

```sh
# Every 15 minutes:
curl -s -X POST http://127.0.0.1:7733/op/schedule_add \
    -H 'Content-Type: application/json' \
    -d '{"cron_expr":"0 */15 * * * *","command":["check-mail.sh"]}'
# {"ok":true,"schedule_id":1,"cron_expr":"0 */15 * * * *"}

# Daily at 03:00:00 with custom env + cwd:
curl -s -X POST http://127.0.0.1:7733/op/schedule_add \
    -H 'Content-Type: application/json' \
    -d '{
      "cron_expr":"0 0 3 * * *",
      "command":["./backup.sh"],
      "cwd":"/home/me",
      "env":{"BACKUP_TARGET":"s3://my-bucket"},
      "notes":"nightly backup"
    }'

# One-shot fire at a specific epoch second:
NOW=$(date +%s); FIRE=$((NOW + 3600))
curl -s -X POST http://127.0.0.1:7733/op/schedule_add_once \
    -H 'Content-Type: application/json' \
    -d "{\"fire_at_unix_secs\":$FIRE,\"command\":[\"echo\",\"hello in 1h\"]}"

# List + remove:
curl -s -X POST http://127.0.0.1:7733/op/schedule_list \
    -H 'Content-Type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:7733/op/schedule_remove \
    -H 'Content-Type: application/json' -d '{"id":1}'
```

## Streaming endpoints (Server-Sent Events)

Two SSE endpoints push notifications to long-lived HTTP clients without
polling. Each event is a `text/event-stream` record:

```
event: <kind>
data: <json>

```

`curl -N` keeps the connection open. JS `EventSource` works the same way.

### File-system watch — `GET /stream/watch?path=DIR&recursive=BOOL`

Subscribes to fsnotify on DIR. Events arrive as `event: fs`.

```sh
# Stream changes under ~/src as JSON paths:
curl -sN 'http://127.0.0.1:7733/stream/watch?path=/Users/me/src&recursive=true' \
  | sed -n '/^data:/s/^data: //p' \
  | jq -r .trigger_path
```

Browser:

```js
const es = new EventSource('http://127.0.0.1:7733/stream/watch?path=/tmp/x');
es.addEventListener('fs', e => console.log('changed:', JSON.parse(e.data)));
```

### Pubsub events — `GET /stream/events?channel=PATTERN`

Subscribes to the daemon's pubsub bus. PATTERN is `<scope>.<topic>`
(default `*.*` = everything). `op_publish` from any client routes to
matching subscribers.

```sh
# Listener:
curl -sN 'http://127.0.0.1:7733/stream/events?channel=*.build_done'

# Publisher (separate shell):
curl -s -X POST http://127.0.0.1:7733/op/publish \
    -H 'Content-Type: application/json' \
    -d '{"topic":"build_done","data":{"target":"prod","status":"ok"}}'
```

The listener receives:
```
event: pub
data: {"data":{"target":"prod","status":"ok"},"scope":"shell:N","subscription_id":null,"topic":"build_done"}
```

## Error responses

| HTTP | `code` field | When |
|---|---|---|
| `200` | n/a — `ok:true` | success |
| `400` | `bad_args` | malformed body, missing required field |
| `401` | `unauthorized` | missing or wrong bearer token |
| `404` | `unknown_op` | no such op (typo, wrong daemon version) |
| `404` | `no_such_file` / similar | op-specific not-found |
| `500` | varies | daemon-internal failure (see `msg`) |

Body always `{"ok":false,"code":"...","msg":"..."}` for any non-200.
