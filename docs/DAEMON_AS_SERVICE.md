# DAEMON_AS_SERVICE.md — zshrs-daemon as a universal user-space service

**Status:** design  
**Owner:** MenkeTechnologies  
**Layer:** singleton daemon + versioned public IPC contract + multi-language client libraries  
**Companion docs:** [DAEMON.md](DAEMON.md) (internal architecture), [RECORDER.md](RECORDER.md) (one application of the daemon)

## Executive summary

`zshrs-daemon` is **not** an internal helper for the zshrs shell. It is
a standalone user-space service exposing a versioned public API that
any tool — any shell (bash, zsh, fish, nushell, elvish, pwsh, ksh,
dash), any editor (vim, emacs, vscode, helix, jetbrains), any
language runtime (Rust, Python, Node, Go, Ruby, Perl), any CI system,
any container — can consume. The daemon collapses six categories of
single-purpose user-space infrastructure into one process:

| Category | Standalone tool today | Subsumed by daemon |
|---|---|---|
| Persistent local key-value cache | Redis (local), memcached | `daemon.cache.{get,put,del,list}` |
| Content-addressed build/artifact cache | sccache, ccache, cargo local cache | `daemon.artifact.{get,put,gc}` |
| Job queue / task supervisor | sidekiq, celery, RQ, pueue, systemd-user-units | `daemon.job.{submit,poll,cancel,logs}` |
| Scheduled jobs | cron, anacron, systemd-user-timers, launchd | `daemon.schedule.{add,remove,list}` |
| File-watcher trigger | inotify-tools, watchman, entr | `daemon.watch.subscribe` |
| Cross-process event bus | dbus, Unix signals (limited) | `daemon.event.{publish,subscribe}` |
| Named cross-process locks | flock(1), filelock libs | `daemon.lock.{acquire,release,try}` |
| Shell-state catalog | nothing exists | `daemon.definitions.{query,diff,subscribe}` |
| Portable shell-snapshot artifact | nothing exists | `daemon.snapshot.{save,load,sign,publish}` |
| Cross-shell coordination | nothing exists | `daemon.shell.{send,broadcast,list}` |

The result: a single, persistent, tokio-backed user-space process
that other tools talk to instead of each one reinventing its own
cache/queue/state/event layer. **K8s for the single-user machine —
without the operational complexity of K8s.**

## The decoupling thesis: daemon ≠ shell client

The most important strategic property: **adoption of zshrs-daemon
does not require adoption of the zshrs shell.** The two ship and
evolve independently. A user can adopt the daemon TODAY while
keeping their existing zsh/bash/fish/nu shell. The migration path
is incremental, not all-or-nothing.

```
                 ┌─────────────────────────────────────┐
                 │         zshrs-daemon                │
                 │  (singleton user-space service)     │
                 └────┬────────┬────────┬────────┬─────┘
                      │        │        │        │
                ┌─────▼──┐ ┌───▼──┐ ┌──▼───┐ ┌──▼────────┐
                │ zshrs  │ │ bash │ │ fish │ │  vim/lsp  │
                │ shell  │ │      │ │      │ │           │
                └────────┘ └──────┘ └──────┘ └───────────┘
                  ▲             ▲      ▲           ▲
                  │             │      │           │
                  │             └──────┴───────────┘
                  │             (existing-shell holdouts —
                  │              same benefits TODAY)
                  │
            (full per-definition recorder integration;
             other shells get capture-via-introspection
             until they migrate, lose only file:line precision)
```

This decoupling means:

1. **The daemon is shippable today.** Every benefit of the
   service-layer architecture (cache, jobs, scheduler, pubsub,
   locks, snapshot artifacts, cross-shell coordination) is
   available to bash/zsh/fish/nu users without waiting for the
   zshrs shell port to complete.
2. **zshrs shell adoption is incremental, not gated.** Users
   adopt the daemon for cache/jobs benefits; the daemon proves
   value; zshrs shell becomes the obvious next step when ready.
3. **The non-zshrs holdout case is first-class.** Designs are
   evaluated against bash + fish + vanilla zsh users, not just the
   zshrs target. This forces the API to be language-agnostic and
   shell-agnostic from day one.
4. **The daemon stands as an independent product.** Even if zshrs
   the shell never reaches 100% zsh compatibility, the daemon's
   value is intact. This is risk reduction for the overall project.
5. **Network effects start before the shell ships.** Daemon
   users → telemetry → feedback → API refinement → reference
   client libraries → adoption snowball — all happening in
   parallel with zshrs-shell development, not after it.

## What every shell user gets today

The daemon is daily-driver-ready for the cache/job/scheduler
subset right now. Concrete capability matrix per shell:

| Capability | bash | vanilla zsh | fish | nushell | zshrs |
|---|---|---|---|---|---|
| `daemon.cache.*` (persistent local KV) | ✅ via `zd` | ✅ | ✅ | ✅ | ✅ |
| `daemon.artifact.*` (build artifact cache, sccache replacement) | ✅ | ✅ | ✅ | ✅ | ✅ |
| `daemon.job.*` (background task queue) | ✅ | ✅ | ✅ | ✅ | ✅ |
| `daemon.schedule.*` (cron replacement) | ✅ | ✅ | ✅ | ✅ | ✅ |
| `daemon.watch.*` (fsnotify-driven triggers) | ✅ | ✅ | ✅ | ✅ | ✅ |
| `daemon.event.*` (cross-process pub/sub) | ✅ | ✅ | ✅ | ✅ | ✅ |
| `daemon.lock.*` (named cross-process locks) | ✅ | ✅ | ✅ | ✅ | ✅ |
| `daemon.shell.send` (cross-shell messaging) | ✅ | ✅ | ✅ | ✅ | ✅ |
| `daemon.snapshot.save` (capture state via introspection) | partial — names + values, no file:line | partial | partial | partial | full — file:line + fn_chain |
| `daemon.definitions.query` (shell-state catalog) | partial | partial | partial | partial | full |
| Recorder per [RECORDER.md](RECORDER.md) | not available — requires AOP intercept | not available | not available | not available | ✅ |

**The 8 capabilities marked ✅ across all shells are universally
available the day the daemon ships its public API.** No new shell
needed.

### Two ways to drive the daemon from any shell

| Surface | Install | Best for |
|---|---|---|
| `zd` binary (`bins/zd.rs`) | `cargo install --path . --bin zd --features zd` | Power users, scripts, editor integrations — single-purpose ~3MB binary, no shell dependency, structured arg parsing, exit codes |
| `daemon-shell.<shell>` wrappers (`examples/daemon-shell.{sh,bash,zsh,fish,nu,elv,ksh,ps1}`) | `source examples/daemon-shell.bash` (or your shell) | Shell users — zero install, idiomatic per shell (`daemon-cache-put` / `daemon record alias` / `Daemon-RecordAlias`), 8 shells covered |

Both target the same HTTP listener and accept identical env vars:
`DAEMON_URL`, `DAEMON_TOKEN`, `DAEMON_SHELL_ID`. Pick whichever fits
the use site — they're interchangeable. The doc below uses `zd` in
examples; substitute `daemon-cache-get` / `daemon record alias` /
etc. if you're going wrapper-only.

The recorder + per-definition file:line attribution is the only
capability that requires the zshrs shell, because it's the only
shell with global runtime AOP across builtin dispatchers (per
[RECORDER.md](RECORDER.md) "Why this can only exist in zshrs").
Non-zshrs shells get a degraded form via shell introspection
(`alias`, `declare -f`, `set -o`, `bindkey -L`, `compdef -d`)
that captures NAMES + VALUES but loses FILE + LINE.

## Universal client model

Every client speaks the same versioned IPC over the same unix
socket. From bash:

```bash
# Persistent KV
zd cache put build-config "$(cat config.json)"
zd cache get build-config

# Job submission
job_id=$(zd job submit "long-running-task.sh")
zd job poll "$job_id"

# Cross-process lock around critical section
zd lock acquire deploy-mutex --timeout 30s
trap 'zd lock release deploy-mutex' EXIT
# ... critical section ...

# fsnotify-driven trigger
zd watch subscribe ~/src/myproject/**/*.rs |
    while read changed; do cargo check; done

# Cross-shell pub/sub
zd event publish build-complete '{"target":"prod"}'

# Build artifact cache (sccache-equivalent for arbitrary tools)
hash=$(sha256sum input.c | cut -d' ' -f1)
if ! zd artifact get "compile-$hash" -o output.o; then
    gcc -c input.c -o output.o
    zd artifact put "compile-$hash" output.o
fi
```

Same operations from fish:

```fish
zd cache put env-snapshot (env)
set job_id (zd job submit "deploy.sh")
```

Same from Python:

```python
from zshrs_client import Daemon
d = Daemon()
d.cache.put("env-snapshot", json.dumps(os.environ))
job = d.job.submit(["python", "long_task.py"])
d.job.poll(job.id)
for path in d.watch.subscribe("~/src/*.py"):
    subprocess.run(["pytest", path])
```

Same from Rust:

```rust
let d = zshrs_client::Daemon::connect()?;
d.cache().put("env-snapshot", &env_dump)?;
let job = d.job().submit(&["./build.sh"])?;
job.wait()?;
```

Same from a vim plugin (rust-mode embedding LSP):

```vim
" Define alias goto-definition for shell-script user
nnoremap gd :call ZshrsGotoDefinition(expand('<cword>'))<CR>

function! ZshrsGotoDefinition(name)
    let result = system('zd definitions query alias '..a:name..' --json')
    let r = json_decode(result)
    if !empty(r)
        execute 'edit ' .. r.file
        execute r.line
    endif
endfunction
```

The protocol is the same; client libraries differ only in idiomatic
language wrapping.

## Public API contract

### Versioning

Semver on the protocol (independent of daemon version). Server
advertises the highest protocol version it supports during the
handshake; clients pin their compatibility range:

```
zshrs-daemon protocol versions:
  v1.0   — initial public API (cache, job, schedule, watch, event,
           lock, definitions, snapshot, shell)
  v1.1   — adds artifact cache, namespace quotas (back-compat)
  v2.0   — breaking changes; daemon supports v1.x and v2.x in parallel
           for at least 12 months overlap
```

Backward-compatible changes only within a major. Breaking changes
trigger a new major; the daemon continues serving the old major
for a minimum 12-month deprecation window.

### Op surface (initial v1.0 — public stable)

Op names are flat snake_case (the dotted `daemon.cache.put` form in
the table below is a logical grouping; the wire op is `cache_put`).
HTTP route: `POST /op/<NAME>` for every op below. SSE streaming
endpoints listed under §"Streaming endpoints" further down.

Status legend:
- ✅ shipped (live in the daemon today, exercised by `tests/daemon_http.rs`
  + the `examples/daemon-shell.zsh` wrappers)
- ⏳ deferred (named here for forward compatibility; not yet implemented)

```
HEALTH                                                                       Status
  GET  /health                            → { ok, version, uptime_ms }                   ✅
  GET  /metrics  (Prometheus 0.0.4 text)  → daemon_uptime_seconds, daemon_op_total{op}…  ✅
  metrics                                 → JSON: { uptime_seconds, op_total{}, … }      ✅

CACHE — persistent KV per client namespace, sqlite-backed
  cache_put         {ns, key, value, ttl_secs?}      → {bytes, expires_at}               ✅
  cache_get         {ns, key}                        → {value, bytes, …} | 404           ✅
  cache_del         {ns, key}                        → {deleted: bool}                   ✅
  cache_list        {ns, prefix?}                    → {keys[], count}                   ✅
  cache_stats       {ns?}                            → {key_count, byte_count, …}        ✅

ARTIFACT — content-addressed cache (sha256 + names.db index)
  artifact_put              {name, value | value_base64}  → {digest, bytes}              ✅
  artifact_get              {name}                        → {digest, value_base64} | 404 ✅
  artifact_get_by_digest    {digest}                      → {digest, value_base64} | 404 ✅
  artifact_gc               {max_age_secs?, max_bytes?}   → {removed, freed_bytes}       ✅
  artifact_list             {prefix?}                     → {entries[], count}           ✅

JOB — tokio-backed task queue with persistent state
  job_submit  {command[], cwd?, env?, tags?}    → {job_id}                                ✅
  job_status  {id}                              → {job: { state, exit_code, … }}          ✅
  job_output  {id, stderr?}                     → {content, stderr: bool}                 ✅
  job_list    {state?, tag?, limit?}            → {jobs[], count}                         ✅
  job_kill    {id}                              → {killed: bool}                          ✅
  job_cancel  {id}                              → {cancelled: bool}                       ✅
  job_wait    {id}                              → {state, exit_code}                      ✅

SCHEDULE — cron-equivalent with persistent state (cache.db / table `schedule`)
  schedule_add        {cron_expr, command[], cwd?, env?, notes?}  → {schedule_id}         ✅
  schedule_add_once   {fire_at_unix_secs, command[], …}            → {schedule_id}        ✅
  schedule_remove     {id}                                          → {removed: bool}     ✅
  schedule_list       {enabled_only?}                               → {schedules[]}       ✅
  // tick driver runs once / second; due rows fire job_submit with tag `scheduled`.

WATCH — fsnotify push stream
  GET /stream/watch?path=DIR&recursive=BOOL  → SSE `event: fs`                            ✅
                                               data: { trigger_path, source_root, … }
  watch_subscribe   {path, recursive?}        → {watch_id, path, recursive}              ✅
  watch_unsubscribe {watch_id}                → {removed: bool}                          ✅
  watch_list        {}                        → {subscriptions[{watch_id, path,
                                                                 ref_count}], count}     ✅
  // Refcounted: same path subscribed N times stays armed until the
  // Nth unsubscribe. HTTP /stream/watch uses watch_subscribe internally
  // and releases on TCP close (no SSE-disconnect leak).

EVENT — user-defined pubsub bus
  publish      {topic, data}                  → {delivered_to: N}                         ✅
  subscribe    {pattern}                      → {subscription_id, pattern}                ✅ (socket)
  unsubscribe  {id}                           → {removed: bool}                           ✅
  GET /stream/events?channel=PATTERN          → SSE `event: pub`                          ✅
                                                data: { topic, data, scope, … }
  // Pattern is `<scope>.<topic>` (e.g. `*.*`, `shell:5.build_done`).

LOCK — named cross-process mutual exclusion (PID-tied auto-release)
  lock_acquire     {name, pid, timeout_secs?}   → {token} | timeout                       ✅
  lock_try_acquire {name, pid}                  → {token} | busy                          ✅
  lock_release     {name, token}                → {released: bool}                        ✅
  lock_list        {}                           → {locks[{name, holder_pid, alive, …}]}   ✅

DEFINITIONS — shell-state catalog (recorder-fed, federated by shell_id)
  definitions_kinds        {}                              → {kinds[], all_known[]}      ✅
  definitions_query        {kind?, name?, prefix?, shell_id?, limit?}                    ✅
                                                           → {records[], count}
  definitions_emit         {shell_id, kind, name, value?, file?, line?, fn_chain?}       ✅
                                                           → {wrote_rows, …}
  definitions_diff         {shell_a, shell_b, kind?}       → {added, removed, changed}   ✅
  definitions_subscribe    {}   → {subscribed: true, was_subscribed: bool}               ✅
  definitions_unsubscribe  {}   → {subscribed: false, was_subscribed: bool}              ✅
  GET /stream/definitions                                   → SSE `event: defs`          ✅
                                                             (fired on every recorder_ingest)
  // recorder_ingest now broadcasts to opted-in sessions only — IPC
  // clients call definitions_subscribe to start receiving the
  // `recorder_ingested` Frame::Event. HTTP /stream/definitions auto-
  // subscribes its synthetic session.

SNAPSHOT — portable canonical-state artifacts
  snapshot_save  {tag, notes?}     → {tag, path, bytes, generation, total_rows}           ✅
  snapshot_list  {}                → {snapshots[{tag, path, bytes}]}                      ✅
  snapshot_load  {tag}             → {rows_restored} (atomic replace_subsystem)           ✅
  snapshot_diff  {a, b}            → {added[], removed[], changed[]}                      ✅
  snapshot_bisect / publish / pull / sign / verify                                       ⏳

SHELL — cross-shell coordination (extends zsend/znotify/zsubscribe)
  list_shells   {}                            → {shells[{id, pid, tty, …}]}               ✅
  send          {target_id, msg, …}           → {delivered}                               ✅
  notify        {message, urgency, …}         → {fanout}                                  ✅
  tag / untag   {label}                       → {tags}                                    ✅

EXPORT — universal cache dump in any format including PDF
  export        {target, format}              → {body | body_base64}                      ✅
  view          {target, format?}             → human-readable dump                       ✅
  // format ∈ sh|json|yaml|text|csv|sql|native|disasm|zcompdump|zsh-histfile|pdf
```

Most ops mirror the IPC surface; the daemon dispatches HTTP and
unix-socket calls through the same `ops::dispatch` function so wire
formats are transport-only differences.

### Streaming endpoints (Server-Sent Events)

| Endpoint | Event kind | Payload | Source op |
|---|---|---|---|
| `GET /stream/watch?path=DIR&recursive=BOOL` | `fs` | `{trigger_path, source_root, shard, …}` | fsnotify debouncer |
| `GET /stream/events?channel=PATTERN` | `pub` | `{topic, data, scope, subscription_id}` | `publish` |
| `GET /stream/definitions` | `defs` | `{events_ingested, rows_written, elapsed_ms, …}` | `recorder_ingest` |

Each connection registers a synthetic IPC session for its lifetime;
disconnect (TCP close) auto-deregisters. Keep-alive comments are
emitted every 15 seconds so HTTP intermediaries don't drop idle
connections. CORS is **not** enabled in v1 (same-origin tooling only;
add a reverse proxy for browser pages).

### Implementation status snapshot (this document, real time)

| Surface | Done | Notes |
|---|---|---|
| HEALTH | ✅ | `/health`, `/metrics` (Prom text), `metrics` op (JSON) |
| CACHE | ✅ | sqlite-backed, TTL-aware, namespaced |
| ARTIFACT | ✅ | sha256 dedup, base64 wire encoding, GC by age + size cap |
| JOB | ✅ | tokio supervisor, per-job stdout/stderr files, terminal states `exited`/`failed`/`killed`/`cancelled` |
| SCHEDULE | ✅ | cron 6-field format, sqlite-persisted, 1Hz tick, fires `job_submit` with `tags:["scheduled"]` |
| WATCH | ✅ | refcounted per-path subscription via `watch_subscribe` (IPC) or `/stream/watch` (HTTP); same path subscribed N times stays armed until the Nth unsubscribe; SSE TCP-close auto-releases |
| EVENT | ✅ | scope.topic patterns, `publish` requires session (HTTP `handler_op` registers per request) |
| LOCK | ✅ | named mutex, u128 token, PID liveness probe |
| DEFINITIONS | ✅ | `kinds`, `query`, `emit`, `diff`, `subscribe`, `unsubscribe`; federated by `shell_id` (composite-key store keeps per-shell rows distinct); IPC subscribe is opt-in (silent clients don't get every recorder bundle); HTTP `/stream/definitions` auto-subscribes; see `docs/SHELL_IDS.md` for identifier registry |
| SNAPSHOT | ✅ (publish/sign deferred) | save/list/load/diff via rkyv `CanonicalShard` |
| SHELL | ✅ | pre-existing IPC ops surfaced over HTTP |
| EXPORT (PDF) | ✅ | `printpdf`-rendered, base64-wire |
| AUTH | ✅ | bearer tokens in `daemon.toml` (legacy flat string OR scoped table form); per-token scopes enforced against op→scope table in `daemon/auth.rs`; refuses non-loopback bind without tokens; unscoped tokens grant full access (backward compat) |
| METRICS | ✅ | Prom 0.0.4 text + JSON op |

Deferred to a follow-up round, named here for forward compatibility:
- `snapshot_bisect / publish / pull / sign / verify`
- per-namespace cache quotas, daemon-wide rate limits
- artifact streaming for large blobs (octet-stream response)
- OpenAPI / protobuf / cddl schema generation

### Authentication / authorization

**Defaults — single-user trust model.** The IPC unix socket is
0600-mode in the user's own cache dir. The HTTP listener is
**disabled by default** and refuses to bind anywhere except loopback
unless at least one bearer token is configured (see
`daemon/http.rs:95`). For solo use, leave `[http.tokens]` empty —
loopback HTTP needs no auth.

**Multi-client opt-in.** Configure bearer tokens in
`~/.zshrs/daemon.toml`. Two value shapes per token:

```toml
[http]
listen = "127.0.0.1:7733"

[http.tokens]
# Legacy / unscoped — flat string. Token grants full access (every op).
mybox      = "0123abcd..."

# Scoped — inline table. Token only grants the listed scopes; every
# other op returns 403 with code `scope_denied`.
vim-lsp    = { token = "feedface...", scopes = ["defs.read", "snapshot.read"] }
ci-pipe    = { token = "deadbeef...", scopes = ["job.write", "cache.*"] }
dashboard  = { token = "00112233...", scopes = ["*.read"] }
admin      = { token = "ffaa9988...", scopes = ["*"] }
```

**Scope namespaces.** Every op maps to a single `<area>.<verb>` scope
in `daemon/auth.rs:op_scope`. The areas are `cache`, `job`, `lock`,
`defs`, `snapshot`, `artifact`, `schedule`, `event`, `watch`, `shell`,
`recorder`, `history`, `ask`, `export`, `import`, `meta`. Verbs are
typically `read` / `write` / `admin` / `control`. Add an op to that
table when you add a new op — unmapped ops fall through to
`meta.admin` (deny-by-default for any new op until explicitly mapped).

**Pattern syntax in `scopes`:**
- `*`            — every op
- `<area>.*`     — every verb in `<area>` (`cache.*`, `job.*`, …)
- `*.<verb>`     — every area's `<verb>` (`*.read`, `*.write`)
- `<area>.<verb>` — exact match (`cache.put`, `defs.read`)

No nested globs, no regex — readability over flexibility.

**403 response on scope mismatch:**

```json
{
  "ok": false,
  "code": "scope_denied",
  "msg": "token `vim-lsp` lacks scope `cache.write` for op `cache_put`",
  "required_scope": "cache.write",
  "granted_scopes": ["defs.read", "snapshot.read"]
}
```

Token files are local-FS-readable; intentional for the single-user
use case (no secret-management infra needed). For multi-machine
deployments, the daemon sits behind whatever reverse proxy /
identity layer the operator wants in front of `127.0.0.1:7733`.

### Wire format

Two formats supported in v1.0:

| Format | Use case | Trade-off |
|---|---|---|
| Length-prefixed CBOR (default) | high-perf clients (Rust, C, Go) | binary, smaller frames, schema-evolution-friendly |
| Length-prefixed JSON-RPC 2.0 | universal interop (shells via `nc`/`curl`, Python/Node/Ruby with stdlib only) | text, larger frames, human-debuggable |

Server speaks whichever the client opens with; no versioned format
selection beyond the handshake.

## Comparison to existing user-space service infra

### Single-purpose tools the daemon collapses

| Tool | Lines of config + cost | Daemon equivalent |
|---|---|---|
| Redis (local install) | systemd unit + redis.conf + 50MB RAM dedicated | `daemon.cache.*` — uses already-running daemon, sqlite-backed |
| sccache | env vars + per-language config + S3 backend setup for sharing | `daemon.artifact.*` — single shared cache for ALL languages |
| sidekiq / celery | Ruby/Python venv + Redis + worker config | `daemon.job.*` — tokio runtime in one process |
| cron / anacron | crontab syntax + system service + log file path | `daemon.schedule.*` — same syntax, single daemon |
| systemd-user-timers | unit files in ~/.config/systemd/user + systemctl --user | same |
| watchman | Facebook tool + per-project config + JSON socket | `daemon.watch.*` — already wired |
| dbus | system-wide daemon + complex API + service files | `daemon.event.*` — simpler model, single-user scope |
| flock(1) | per-script flock + careful trap cleanup | `daemon.lock.*` — named, automatic cleanup on PID exit |
| pueue (Rust) | dedicated daemon + CLI; no integration with anything else | rolled into daemon.job + daemon.schedule |
| Total | ~10 separate processes, ~10 config files, ~500MB combined RAM | one process, one config, ~50MB RAM |

### Closest existing precedents and why none ship the full surface

| System | What it does | What it lacks (for this use case) |
|---|---|---|
| systemd --user | per-user service supervision | no general client API, no shell-state, no cache, no event bus, no fsnotify, no Mac/BSD support |
| launchd (macOS) | per-user service supervision | macOS-only, no API for ad-hoc client code, no shell awareness |
| dbus | event bus + service discovery | no scheduling, no cache, no shell-state, complex API |
| Redis (single-machine) | cache + pubsub + simple queue | no scheduler, no shell-state, no fsnotify, no snapshot artifacts |
| sccache | build artifact cache | one purpose only |
| watchman | file-watch service | one purpose only |
| pueue | job queue | one purpose only |
| Nix daemon | build cache + sandboxing | nix-specific |
| Kubernetes (minikube/k3s on single machine) | full orchestration | over-engineered for solo-dev; not shell-aware; high operational cost |

**The unique combination is "all of the above + shell-state catalog + portable shell snapshots, in one ~50MB process, behind a single versioned API, single-user-machine optimized."** No prior art ships this combination.

## Boot-time autostart

The daemon is single-user and stateless across restarts (cache rehydrates
from on-disk shards). Three install paths ship in `examples/`:

| Platform | Mechanism | File | Install command |
|---|---|---|---|
| Linux | systemd user unit | `examples/systemd/zshrs-daemon.service` | `cp …/zshrs-daemon.service ~/.config/systemd/user/ && systemctl --user enable --now zshrs-daemon` |
| macOS | launchd LaunchAgent | `examples/launchd/com.menketechnologies.zshrs-daemon.plist` | `examples/install-launchd.sh` (templates `$HOME` + loads with launchctl) |
| Both | Homebrew formula service stanza | `examples/brew/zshrs.rb` | `brew tap MenkeTechnologies/tap && brew install zshrs && brew services start zshrs` |

The systemd unit needs `loginctl enable-linger $USER` for the daemon to
survive logout. The launchd LaunchAgent runs at user-login automatically
(`RunAtLoad=true`), no extra flag needed. `brew services` materializes
the formula's `service` block into the platform's native unit (launchd
on macOS, systemd-user on Linux) so one command works on both.

## What this enables — usage scenarios across shells

### Scenario 1: bash user, no zshrs shell, daemon adopted today

```bash
# .bashrc additions
export PATH=$HOME/.cargo/bin:$PATH      # zd installed via cargo

# Replace ~/.local/bin/sccache with daemon-backed cache
export RUSTC_WRAPPER=zd-rustc-wrapper

# Replace cron + anacron for personal jobs
zd schedule add "0 */1 * * *" "backup-photos.sh"
zd schedule add "0 3 * * *" "git-pull-all-repos.sh"

# Replace per-script flock for deploy mutex
deploy() {
    zd lock acquire prod-deploy --timeout 60s || return 1
    trap 'zd lock release prod-deploy' EXIT
    # ... actual deploy ...
}

# Cross-shell notifications without external service
build-and-notify() {
    if cargo build; then
        zd event publish build "{ \"status\": \"ok\", \"target\": \"$1\" }"
    fi
}

# Listen in another shell
zd event subscribe build | while read msg; do
    notify-send "build" "$msg"
done

# Capture current bash state for diffing later
zd snapshot save --tag laptop-bash-2026-05-02
```

User has not changed shells. Has gained 5 capabilities that
previously required Redis + cron + watchman + dbus + custom
notification setup.

### Scenario 2: fish user with bare-metal install

```fish
# fish-flavored config
function on-rust-change
    zd watch subscribe '~/src/**/*.rs' | while read changed
        echo "rebuild on $changed"
        cargo check
    end
end

# fish universal variables for tagged daemon namespaces
set -U DAEMON_NS my-fish-env

# Save fish env state
zd snapshot save --tag fish-baseline
```

### Scenario 3: vim editing a shell script, jump-to-definition

```vim
" .vimrc — works regardless of which shell user runs interactively
nnoremap gd :ZshrsGotoDef<CR>

command! ZshrsGotoDef call s:goto_def()
function! s:goto_def() abort
    let name = expand('<cword>')
    let json = system('zd definitions query --any --name '..shellescape(name)..' --json')
    let recs = json_decode(json)
    if empty(recs) | echo 'no definition' | return | endif
    " Pick most-recent record
    let r = recs[0]
    execute 'edit '..r.file
    call cursor(r.line, r.col)
endfunction
```

User editing a shell script with vim hits `gd` on `gst` and lands
at `~/.zpwr/env/.shell_aliases_functions.sh:1742`. Editor doesn't
need to parse zsh, doesn't need to know what plugin manager
loaded the alias, doesn't need a language server for shell —
just queries the daemon.

### Scenario 4: Python data-processing script wants to know what aliases the user has

```python
from zshrs_client import Daemon

d = Daemon()
aliases = d.definitions.query(kind="alias")
# [{"name": "gst", "value": "git status -sb", "file": "...", "line": 1742, ...}, ...]

# Generate a markdown table of all aliases for documentation
for a in sorted(aliases, key=lambda x: x.name):
    print(f"| `{a.name}` | `{a.value}` | {a.file}:{a.line} |")
```

Today: this requires `subprocess.run(['zsh', '-i', '-c', 'alias'])`
which is slow, fragile, and gives strings, not structured data.

### Scenario 5: CI pipeline asserts shell environment

```yaml
# .github/workflows/deploy.yaml
- name: Verify deploy environment
  run: |
    zd expect 'alias deploy exists' \
                     'PATH contains /opt/homebrew/bin' \
                     'export NODE_ENV=production' \
                     --snapshot blessed-deploy-shell-v3.rkyv
```

Pipeline fails if shell environment doesn't match the blessed
snapshot. Today: Bash + grep + custom assertion scripts; brittle.

### Scenario 6: Migration from bash → zshrs over months

```
Month 0:  Install daemon + zd. Keep using bash. Adopt:
            - zd cache (replaces in-house Redis usage)
            - zd schedule (replaces cron)
            - zd lock (replaces flock per-script)

Month 2:  Snapshot bash environment via introspection (names+values, no file:line)
          Use snapshot for cross-machine sync via daemon.snapshot.publish

Month 4:  Install zshrs shell binary alongside bash
          Run `zshrs-recorder` to capture full per-definition catalog
          (file:line + fn_chain now available)
          
Month 6:  Make zshrs the login shell. Existing daemon ops continue working.
          All previously-set up snapshots, schedules, jobs still active.

Month 8:  Discover full capability surface. Use zwhere queries that were
          partial before zshrs was installed.
```

The daemon survives every transition. No migration cost on the
service-layer side.

## Third-party shell recorders: the federated catalog model

The daemon contract is shell-agnostic by design. Each shell can ship
its own recorder against the same catalog without zshrs project
involvement. The result is a **federated catalog**: zshrs, fish, bash,
vanilla zsh all writing to one shared daemon-owned definitions
table, queryable across shells via the public API.

### Why this is the design intent, not an afterthought

The public ops `daemon.definitions.{query,subscribe}` and `daemon.snapshot.*`
take records keyed by `(kind, name, file, line, fn_chain, ts_ns,
shell_id)`. The `shell_id` field carries the recording shell's
identity (zshrs-recorder, fish-recorder, bash-recorder, etc.) so queries can
filter by shell or aggregate across:

```
# All aliases defined anywhere across all shells:
zd definitions query --kind alias

# Just fish-recorder records:
zd definitions query --kind alias --shell-id fish

# Cross-shell diff: aliases in fish but not in zshrs:
zd definitions diff --kind alias --shell-a fish --shell-b zshrs
```

The schema explicitly accommodates third-party recorders. The op
surface does not require any zshrs-specific knowledge to emit a
record; any shell with file:line introspection + a way to call
into the daemon (via `zd` or HTTP-bridge) can contribute.

### What fish-recorder would look like

Fish's primitives map to recorder intercept points without requiring
fish to grow a runtime AOP layer. Fish-recorder is a one-shot tool
that runs in record-mode, shadowing the relevant builtins via
`function function`/`function alias`/etc.:

```fish
# fish-recorder pseudo-source

function alias --no-scope-shadowing
    # Capture file:line BEFORE delegating
    set -l rec_file (status filename)
    set -l rec_line (status line-number)
    set -l rec_chain (status stack-trace | string trim)

    # Emit to daemon
    zd definitions emit \
        --shell-id fish \
        --kind alias \
        --name $argv[1] \
        --value $argv[2] \
        --file $rec_file --line $rec_line \
        --fn-chain "$rec_chain"

    # Delegate to fish's actual alias builtin
    builtin alias $argv
end

# Same pattern for: function, abbr, set, bind, complete, source
```

Fish ships this as `~/.config/fish/conf.d/zshrs-recorder.fish` (only
loaded when `fish-recorder` mode is active via env var).

### Fidelity tier across shells

| Shell | Recorder mechanism | File:line | Slowdown during recording | Fidelity vs zshrs-recorder |
|---|---|---|---|---|
| zshrs | runtime AOP at every state-mutating dispatcher | full | <0% | 100% (reference) |
| fish | function-shadowing + `status filename` / `line-number` | full | ~5-10× | ~95% (perf-only gap; data is full) |
| zsh (vanilla) | function-shadowing + `funcfiletrace` array | full | ~5-10× | ~95% |
| bash | function-shadowing + `BASH_SOURCE` / `BASH_LINENO` arrays | full, with caveats | ~10-20× | ~85% (assignment-syntax mutations not catchable) |
| nushell | `source` introspection; in-process AOP not exposed | partial | varies | ~70% |
| dash / sh | DEBUG trap only | partial | ~100× | ~30% |
| ksh | typeset discipline + DEBUG trap | partial | ~50× | ~50% |

The performance gap reflects that only zshrs has runtime AOP woven
into the dispatcher fabric; everyone else is using function-shadowing
or trap mechanisms that re-enter the script-evaluator on every call.
Fish achieves the highest fidelity among non-zshrs shells because
its `function --on-event` + `status` introspection give clean intercept
points without C-level changes.

### Strategic value of third-party recorders

1. **Federated catalog.** Same daemon accepts records from any
   recorder. Cross-shell queries become possible (`which shells have
   alias gst defined?`). Single source of truth for shell state
   across a user's entire shell environment, regardless of how many
   shells they run.
2. **Project-scope reduction for zshrs.** zshrs ships ONE recorder
   (the AOP-instrumented one). The bash/fish/zsh communities ship
   THEIR recorders. zshrs doesn't have to write 6 different recorders
   for 6 shells.
3. **Shell-ecosystem network effect.** Each new third-party recorder
   = more daemon adoption = stronger pull for the next community to
   ship one. Same flywheel that drove LSP from a Microsoft tool
   into a universal editor protocol.
4. **Empirical validation of the shell-agnostic claim.** When shells
   other than zshrs ship clients against the daemon contract, the
   "shell-agnostic adoption substrate" claim is validated by external
   adoption rather than just by design intent.
5. **Federation enables migration tooling.** "Show me what aliases
   I'd lose if I switched from fish to zshrs" becomes a daemon query
   answerable from any shell. Today: impossible without writing
   custom shell-equivalence checkers.

### Reference protocol for third-party recorders

To make adoption concrete, the project ships:

- **Spec doc:** `docs/RECORDER_PROTOCOL.md` covering the record
  schema, batching rules, shell_id registration, fn_chain encoding,
  sensitive-flag conventions
- **Reference implementations:**
    - `zshrs-recorder` (in-tree, AOP, full fidelity)
    - `fish-recorder` (in-tree, function-shadowing, ~95% fidelity)
    - `bash-recorder` (in-tree, function-shadowing + DEBUG trap, ~85%)
    - These serve as the canonical examples for any other shell
      community wanting to ship their own
- **Conformance test suite:** input shell config → expected catalog
  records, verifiable against any recorder implementation
- **shell_id registry:** `docs/SHELL_IDS.md` reserves identifiers
  (`zshrs`, `fish`, `bash`, `zsh`, `nu`, `elvish`, `pwsh`, `ksh`,
  `dash`, `xonsh`, `oil`/`ysh`, etc.) so third-party recorders
  don't collide

A third party (fish maintainer, bash community contributor) can
ship their recorder by:

1. Reading `RECORDER_PROTOCOL.md`
2. Implementing the intercept layer in their shell's idiomatic style
3. Emitting records via `zd definitions emit` (or directly
   via the language client crate)
4. Reserving a `shell_id` if not already in the registry
5. Running the conformance tests
6. Publishing under their own project namespace (no zshrs-project
   coupling required)

zshrs-project provides the protocol + reference impls + tests + ID
registry. Third parties provide the per-shell intercept code and
ship under their own brand. Everyone writes to the same federated
catalog.

## Shell loyalty as TAM expansion, not adoption blocker

Many users will never switch shells. Decade-plus muscle memory,
dotfile investment, framework choice, plugin-ecosystem familiarity,
and explicit preference (fish's interactive UX, bash's POSIX-purity,
nu's structured pipelines) make shell choice durable for a large
fraction of users. Every prior shell-replacement project hit this
wall and failed to scale beyond single-digit-percent share of the
incumbent.

The daemon-first architecture is structurally different: shell
loyalty is not a problem to overcome but a property the
architecture is designed around.

### TAM under each adoption model

| Model | Adoption requirement | Total addressable market |
|---|---|---|
| "Replace your shell with zshrs" (every prior shell-replacement attempt) | Abandon existing shell, dotfiles, plugin manager, muscle memory | migration-willing fraction (1-5% of any incumbent's userbase, historically) |
| "Use the daemon, keep your shell" (zshrs's daemon-first model) | Install one binary; daemon serves any shell via shell-specific recorder | union of zsh + bash + fish + nu + elvish + pwsh + ksh + dash users — every developer with a Unix-like environment |

Every prior shell-replacement attempt failed at the same gate.
fish has been "the better shell" since 2005; after 20 years it
captures single-digit percent of the zsh+bash userbase. nushell
has been "the structured-pipeline shell" since 2019 and sits in
the same range. xonsh, elvish, ion, oil — each captured a small
loyal community and stalled. The convert-everyone-or-fail model
has 30+ years of consistent empirical results.

zshrs's daemon-first architecture skips that trap:

| Group | Outcome under daemon-first model |
|---|---|
| Fish loyalists | Install daemon; fish-recorder ships. 100% of cache/job/scheduler/snapshot/event-bus value with shell unchanged. **Never converts to zshrs the shell. Never needs to.** |
| Bash holdouts | Same model — bash-recorder. Stay on bash forever; daemon still serves them. |
| Vanilla zsh users on macOS | Same — zsh-recorder. Apple ships system zsh; users stay on Apple's. |
| Nushell / elvish / pwsh users | Same — community recorders. |
| Migration-willing minority | Become zshrs adopters; get full per-definition AOP recorder + native parallel execution + AOT trailer + full snapshot artifacts. **The "if you want everything" tier.** |

The 1% who would have switched anyway still switch and become
zshrs users. The 99% who wouldn't have switched contribute daemon
adoption. **Both groups feed the same daemon catalog.** Network
effects compound across the union of all shell users, not just
the migration-willing slice.

### Why shell loyalty is durable

| Source of loyalty | Why it doesn't fade |
|---|---|
| Muscle memory | 10+ years of keyboard reflexes; not transferable in <weeks |
| Dotfile investment | hundreds-to-thousands of lines of personal customization, often grown organically over years |
| Plugin ecosystem familiarity | "I know zinit's ICE modifiers" is not a transferable skill |
| Framework choice | oh-my-zsh, prezto, fisher, oh-my-fish — each implies a worldview |
| Explicit aesthetic preference | fish's autosuggestions, bash's POSIX-purity, nu's data model — chosen for reasons that the user still believes |
| Sunk-cost risk aversion | switching means re-debugging every config decision they already made once |
| Community / advice asymmetry | "search Stack Overflow" works for the dominant shells; switching breaks that |

These are not technical objections that the next-better shell
overcomes. They are durable user properties that the architecture
has to accommodate, not argue with.

### Marketing implications

| Anti-pattern (every prior shell-replacement project) | zshrs's daemon-first positioning |
|---|---|
| "Switch from $YOUR_SHELL to $NEW_SHELL — it's faster / better / nicer" | "Install the daemon. Keep your shell. Get faster cold starts, persistent cache, snapshot artifacts. The shell you already use is fine." |
| Comparison tables that imply the old shell is bad | Comparison tables of what the daemon ADDS to whatever shell you already use |
| "You're stuck on a 30-year-old shell" framing | "Your shell needed a service layer. We built one. Plug it in." |
| Pitch heavy on language / syntax / pipeline differences | Pitch heavy on infrastructure: cache, jobs, scheduler, snapshots, federation |
| Conversion CTA: download the new shell | Adoption CTA: install daemon + shell-specific recorder; current shell unchanged |

Existing-shell users have been pitched "switch to my better shell"
for 30 years and grown immune. Defusing that pattern is the unlock —
the message is "your shell is fine; the missing piece is the
service layer that no shell ever shipped, and now exists."

### Project-positioning implications

1. **fish-recorder, bash-recorder, zsh-recorder are flagship products,
   not charity cases.** They ship from zshrs project, branded as
   "zshrs project's contribution to the $SHELL ecosystem." Same
   priority and quality bar as zshrs-recorder itself.
2. **The daemon's user count > the zshrs shell's user count, permanently.**
   This is the desired ratio, not a failure mode. Daemon installs
   are the leading indicator; zshrs-shell installs are the
   premium-tier-conversion indicator.
3. **Documentation defaults to "for any shell user," not "for zshrs
   users."** Examples in bash, fish, zsh, zshrs in that order across
   tutorial sections. zshrs-specific features land in their own
   sub-sections, not in the default flow.
4. **No anti-fish, anti-bash, anti-zsh framing anywhere in
   marketing.** The shells the daemon serves are partners, not
   competitors. Their loyalists are the customer base.
5. **TAM modeling for the project anchors on daemon adoption,
   not shell-replacement velocity.** Conversion of fish-loyalists
   to zshrs-shell-adopters is not a tracked metric.

### Historical analog: Docker vs OS-replacement projects

The closest precedent in software-infrastructure history:

| Era | OS-replacement projects | Container-infrastructure project (Docker) |
|---|---|---|
| Pitch | "use this OS instead of Linux/Windows/macOS" | "keep your OS; run containers on top" |
| Required user action | adopt new OS, migrate workloads, retrain | install one binary; existing OS unchanged |
| Adoption ceiling | small loyal communities (Solaris, BeOS, Plan 9, Haiku) | universal across every Linux distro + Mac + Windows |
| Outcome | each became a niche or dead | became the deployment layer for ~all of cloud infra |
| Why | demanding OS-replacement is a high-friction migration that competes with the user's existing investment | shipping a service layer that augments the existing OS sidesteps the migration cost entirely |

Pre-Docker: "use this OS instead" failed (Solaris, BeOS, Plan 9,
Haiku, every alt-OS). Post-Docker: "keep your OS, run containers"
succeeded universally. The infrastructure-layer pitch wins; the
user-tier-replacement pitch loses, regardless of how technically
superior the replacement is.

zshrs's daemon-first architecture is the Docker move applied to
shell ecosystems for the first time. Fish loyalists will never
become zshrs-shell users — and they don't have to. They become
daemon users. That is the design goal; it is the architectural
property the patent claim X.B explicitly designed in. The 99%
who don't switch shells are the majority of the daemon's users
and the strategic foundation of the project's TAM.

### The convert-everyone narrative is the failure mode

If a user says "I love fish, I'll never switch," the correct
response is **"good — install the daemon."** Not:

- "You should try zshrs, it's better." (alienates loyalist)
- "Why won't you switch?" (defensive)
- "Fish is fine but..." (qualifier; loyalist hears "fine but
  inferior")
- "OK, you can keep using fish for now." (implies they should
  eventually switch)

The correct response is treating the loyalist's shell choice as
already-correct and pitching the orthogonal layer. The daemon's
value to a fish user has zero overlap with fish's identity. Cache
is not a shell feature. Job scheduling is not a shell feature.
Snapshot artifacts are not a shell feature. The daemon serves
needs the shell does not address; pitching it does not require
relitigating the shell choice.

This is the architectural insight. Every previous shell-replacement
project failed at the conversion ask. zshrs's daemon-first model
removes the ask entirely. Adoption scales with the union of all
shell userbases instead of competing for migration share within
each one.

## Patent-strategy alignment

This adds two more dependent claims to the existing strategy
(per `aot_patent_strategy.md` memory):

**Claim X.A (under daemon foundation B):**

> Method for unified user-space orchestration combining (a) shell-
> state catalog with file:line attribution, (b) general-purpose
> persistent KV cache, (c) content-addressed artifact cache, (d)
> tokio-backed job queue with cron-equivalent scheduling, (e)
> file-watch and cross-process event bus, (f) named distributed
> locks, (g) cross-shell messaging, (h) portable shell-snapshot
> artifact distribution, all served by a singleton tokio-runtime
> daemon process behind a versioned public API contract addressable
> from arbitrary clients including non-zshrs shells, editor
> plugins, language runtimes, CI tools, container runtimes, and
> system monitoring.

**Claim X.B (under same):**

> The shell-agnostic adoption substrate of (X.A): the singleton
> daemon ships and provides public-API value independent of any
> shell client implementation, enabling existing-shell users
> (bash, fish, zsh, nu) to consume cache/job/schedule/event/lock/
> snapshot capabilities without migration to a new shell, with
> graceful degradation in shell-state attribution precision (names
> + values from introspection vs file:line + fn_chain from
> AOP-instrumented zshrs).

The novelty surface includes the COMBINATION of these primitives in one
process AND the decoupling that lets the daemon ship value to
arbitrary shells. No competitor can match by adding one or two
primitives; they must ship the full surface to compete.

## Implementation phases

### Phase 1 — protocol stabilization (1 week)

- Spec v1.0 protocol in `proto/zshrs-daemon-v1.cddl`
- Generate openapi.json + protobuf for client-side codegen
- Write conformance test suite (test against in-process mock daemon)
- Document semver policy + deprecation window
- Versioned handshake op

### Phase 2 — public op surface promotion (2 weeks)

- Audit existing daemon ops (per `daemon/lib.rs::ops`)
- Promote stable ops to public; rename if needed; document schema
- Implement missing ops:
    - `cache.*` (sqlite-backed, namespaced)
    - `artifact.*` (rkyv-backed, content-addressed)
    - `job.*` (formalize zjob_builtin into general API)
    - `schedule.*` (formalize ticker into cron parser + persistent state)
    - `lock.*` (named, with PID-tied auto-release)
- Per-namespace quotas + rate limiting
- Schema validation on every op

### Phase 3 — Rust client crate (1 week)

- Split out `zshrs-client` from existing daemon-internal client code
- Publish to crates.io as `zshrs-client`
- Doc + examples + integration tests
- `zd` binary built on top of the client crate

### Phase 4 — first foreign-language client (Python, 2 weeks)

- `pip install zshrs` package
- PyO3 wrapper around Rust client OR pure-Python protocol impl
- Doc + example notebooks + asyncio support

### Phase 5 — auth / authz hardening (1 week)

- Per-client tokens in `~/.zshrs/daemon.toml`
- Scope enforcement (read-only vs write, per-namespace)
- Optional TCP listener with mutual TLS for cross-machine
- Per-client request quotas + rate limits
- Audit log

### Phase 6 — additional language clients (~1 week each)

- Node.js (`@zshrs/client` on npm) — TypeScript types from openapi
- Go (`github.com/MenkeTechnologies/zshrs-client-go`)
- Ruby (`gem install zshrs-client`)
- C header for editor plugin embedding

### Phase 7 — reference applications (parallel, 2-4 weeks each)

- `zshrs-lsp`: Language Server Protocol reference impl using
  daemon.definitions for shell-script editing
- `zshrs-monitor`: prometheus exporter for system observability
- `zshrs-snapshot-registry`: OCI-compatible registry for shell
  snapshots
- `zshrs-blessed-shells`: corporate distribution example

### Phase 8 — documentation site + brand position

- mkdocs site at `daemon.zshrs.dev` covering protocol, clients,
  use cases, comparisons, migration guides
- Position as "userland k8s for the single-user machine"
- Migration guides per starting shell (bash → daemon, fish →
  daemon, vanilla zsh → daemon, full → zshrs)

**Total: ~10-12 weeks** to ship full public-service launch. Most
of the work is contract stabilization + multi-language clients +
docs; the daemon substrate is already in place.

## Brand position

The daemon is the architectural unlock; the recorder is one
flagship application; the shell-snapshot artifact is a second; the
universal cache/job/event service is a third. Together they
constitute a single product family under one brand:

> **zshrs-daemon: the persistent runtime your shell environment
> should have had since 1971.**

The shell ecosystem has spent 50 years working around the absence
of a singleton service layer for user-space state. Every existing
shell rebuilds its runtime state from source on every cold start
(per the "rebuild your house every morning" critique in the
project conversation). zshrs-daemon is the singleton service all
those shells should have had — finally built, finally accessible,
finally adoptable today, regardless of which shell the user runs.

The decoupling thesis is what makes this realistic: bash and
fish and vanilla zsh users do not have to wait for the zshrs
shell port to finish to benefit. The daemon ships value the day
its public API stabilizes. zshrs-the-shell is the natural endgame
for users who want full per-definition recording + AOT-compiled
startup + parallel execution; zshrs-the-daemon is the universal
substrate everyone can adopt this week.

## Open questions

1. **Should the daemon support non-Unix targets (Windows native)?**
   - WSL handles Linux client compat. Native Windows requires named
     pipes instead of unix sockets. Defer to v2.0.
2. **Should the cache layer support distributed/clustered mode?**
   - Single-machine is the v1.0 target. Multi-machine sync
     (laptop ↔ server) is the v2.0 enhancement; mTLS-based
     daemon-to-daemon replication.
3. **Should the event bus be schema-typed or freeform?**
   - v1.0: freeform JSON-blob payloads. v2.0: optional schema
     declaration per channel.
4. **How does the daemon handle multi-tenant on shared servers?**
   - Per-user daemon (default; PID + socket per user). Shared
     daemons would require kernel-namespace separation; out of
     scope for v1.0.
5. **Should the daemon offer a webhook bridge?**
   - HTTP-receive at `127.0.0.1:PORT/webhook/<channel>` would let
     external services (GitHub, CI, etc.) push events into the
     daemon's event bus. v1.1 feature; trivial on top of the
     existing tokio HTTP listener.
6. **Should snapshots be cryptographically chained (Merkle)?**
   - Each snapshot's manifest references the previous snapshot's
     hash → tamper-evident history. Useful for compliance audits.
     v2.0 feature.

## Non-goals

- Replacing system-level service managers (systemd, launchd) for
  system-level services — those have privileged scope this daemon
  does not need.
- Cross-tenant multi-user isolation on shared servers (separate
  daemon per user is the model).
- Replacing distributed-systems orchestrators (Kubernetes,
  Nomad) — the daemon is single-machine by design.
- Becoming a general-purpose IPC bus replacement for dbus on
  Linux desktops — the daemon's scope is shell + dev-tooling
  user-space, not desktop apps.

## TL;DR

`zshrs-daemon` is a singleton user-space service that subsumes
seven categories of single-purpose infrastructure (KV cache,
artifact cache, job queue, scheduler, file-watch, event bus,
distributed locks) plus three shell-specific capabilities
(state catalog, snapshot artifacts, cross-shell coordination)
into one tokio-backed process behind a versioned public API.

Adoption is **shell-independent**: bash, fish, vanilla zsh, nu,
and zshrs all consume the same API; the daemon ships value to
non-zshrs users on day one. Full per-definition shell-state
recording (file:line + fn_chain) requires the zshrs shell's
AOP layer, but the other 8 capabilities work universally.

Closest precedents (systemd-user, dbus, Redis, sccache, pueue,
launchd) each cover one slice. The unified surface — shell-state-
aware userland k8s for a single machine — is unique to zshrs.

Implementation is ~10-12 weeks on top of the existing daemon
substrate. The substrate is already in place; the work is
contract stabilization + multi-language clients + docs +
reference apps.

The decoupling thesis is the strategic key: **the daemon and
the shell ship and adopt independently**. This eliminates the
all-or-nothing migration cost that every previous shell-replacement
project has been gated on. Users adopt the daemon now; switch
to zshrs the shell when ready; never lose existing infra in
either transition.
