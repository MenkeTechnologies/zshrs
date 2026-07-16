```
 ██████╗  █████╗ ███████╗███╗   ███╗ ██████╗ ███╗   ██╗
 ██╔══██╗██╔══██╗██╔════╝████╗ ████║██╔═══██╗████╗  ██║
 ██║  ██║███████║█████╗  ██╔████╔██║██║   ██║██╔██╗ ██║
 ██║  ██║██╔══██║██╔══╝  ██║╚██╔╝██║██║   ██║██║╚██╗██║
 ██████╔╝██║  ██║███████╗██║ ╚═╝ ██║╚██████╔╝██║ ╚████║
 ╚═════╝ ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝ ╚═════╝ ╚═╝  ╚═══╝
```

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

### `[SINGLETON STATE SERVICE FOR ZSHRS]`

> *"One process owns the disk. Every shell is a client."*

The daemon behind [zshrs](https://github.com/MenkeTechnologies/zshrs) — a single process per user that owns `~/.zshrs/`: fsnotify watching, rkyv shard images, catalog/history/cache SQLite, cross-shell coordination, pub/sub, job supervision, and an optional HTTP listener. Shells attach over a Unix socket and never re-derive state themselves. ~24k lines of Rust across 38 modules.

### [`zshrs`](https://github.com/MenkeTechnologies/zshrs) · [`fusevm`](https://github.com/MenkeTechnologies/fusevm) · [`strykelang`](https://github.com/MenkeTechnologies/strykelang)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Architecture](#0x01-architecture)
- [\[0x02\] Module Map](#0x02-module-map)
- [\[0x03\] On-Disk Layout](#0x03-on-disk-layout)
- [\[0x04\] IPC Protocol](#0x04-ipc-protocol)
- [\[0x05\] Op Surface](#0x05-op-surface)
- [\[0x06\] HTTP Surface](#0x06-http-surface)
- [\[0x07\] Binaries](#0x07-binaries)
- [\[0x08\] Build & Test](#0x08-build--test)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

Every shell startup trick zsh accumulated — `zcompdump`, `zcompile`, zinit turbo,
instant prompt — exists because each interpreter re-derives the same state from
the same files on every launch. The daemon deletes that class of work: one
process resolves state once, keeps it warm, and hands shells a mmap'd image.

- **Singleton.** An `flock` on `daemon.pid` admits exactly one daemon per
  `$ZSHRS_HOME` ([`pidlock.rs`](pidlock.rs)). Second instance exits with
  `AlreadyRunning(pid)`.
- **Spawn-on-demand.** Shells don't require a supervisor; `Client::connect`
  starts the daemon if the socket is cold ([`client.rs`](client.rs)). systemd
  and launchd units exist for those who want them
  ([`examples/systemd/`](../examples/systemd/), [`examples/launchd/`](../examples/launchd/)).
- **fsnotify only, never re-walk.** The daemon watches, debounces, rebuilds the
  affected shard, bumps a generation, atomic-renames, and pushes
  `shard_updated` to subscribers ([`fsnotify.rs`](fsnotify.rs)). Periodic
  re-walking is rejected by design.
- **rkyv is authoritative; SQLite mirrors.** Compiled payloads live in mmap'd
  shards under `images/`. The SQLite databases are queryable mirrors for
  `dbview` and SQL introspection — they never define cache semantics.
- **No AST walk.** State attribution comes from `zshrs-recorder` (runtime AOP
  intercept, see [`docs/RECORDER.md`](../docs/RECORDER.md)), not from parsing
  user config. The daemon services cache ops and fsnotify broadcasts; it never
  re-derives state by reading `.zshrc` ([`lib.rs:90`](lib.rs)).

Full design: [`docs/DAEMON.md`](../docs/DAEMON.md). Service/HTTP deployment:
[`docs/DAEMON_AS_SERVICE.md`](../docs/DAEMON_AS_SERVICE.md).

---

## [0x01] ARCHITECTURE

```
  zshrs shell          zshrs shell          zd / curl / editor LSP
       │                    │                         │
       │  unix socket       │                         │  HTTP (optional,
       │  daemon.sock       │                         │  loopback default)
       └─────────┬──────────┘                         │
                 ▼                                    ▼
          server.rs (tokio accept loop)         http.rs (axum)
                 │                                    │
                 │  Hello/Welcome handshake           │  Bearer + scopes
                 │  u32-BE JSON frames                │  (auth.rs)
                 └────────────────┬───────────────────┘
                                  ▼
                             ops.rs dispatch
                                  │
        ┌────────────┬────────────┼────────────┬─────────────┐
        ▼            ▼            ▼            ▼             ▼
    state.rs      shard.rs    catalog.rs   jobs.rs      pubsub.rs
   sessions      rkyv mmap     history.rs  supervision   broadcast
   tags/registry  images/      cache.rs                  channels
        │            ▲
        │            │ rebuild on change
        ▼            │
    ticker.rs    fsnotify.rs ◄──── fpath / plugin dirs
   housekeeping   debounced
    (1/min)        watcher
```

---

## [0x02] MODULE MAP

| Area | Modules |
|------|---------|
| **Lifecycle** | [`pidlock.rs`](pidlock.rs) singleton flock · [`server.rs`](server.rs) accept loop · [`firstrun.rs`](firstrun.rs) first-run notice · [`log.rs`](log.rs) rolling tracing appender · [`ticker.rs`](ticker.rs) 1/min housekeeping |
| **Transport** | [`ipc.rs`](ipc.rs) framing + handshake · [`client.rs`](client.rs) client helpers + spawn-on-demand · [`http.rs`](http.rs) axum listener · [`auth.rs`](auth.rs) bearer tokens + scopes |
| **Dispatch** | [`ops.rs`](ops.rs) op table · [`builtins.rs`](builtins.rs) z\* builtin IPC wrappers · [`zd_dispatch.rs`](zd_dispatch.rs) transport-agnostic `zd` subcommands |
| **State** | [`state.rs`](state.rs) sessions/tags/registry/broadcast · [`canonical.rs`](canonical.rs) canonical state · [`snapshot.rs`](snapshot.rs) save/load/diff · [`definitions.rs`](definitions.rs) definition index |
| **Storage** | [`shard.rs`](shard.rs) rkyv images · [`catalog.rs`](catalog.rs) · [`history.rs`](history.rs) · [`cache.rs`](cache.rs) · [`artifact.rs`](artifact.rs) content-addressed blobs · [`paths.rs`](paths.rs) layout + 0700/0600 enforcement |
| **Coordination** | [`pubsub.rs`](pubsub.rs) · [`lock.rs`](lock.rs) named locks · [`jobs.rs`](jobs.rs) supervision · [`schedule.rs`](schedule.rs) cron · [`zsync.rs`](zsync.rs) push/pull/diff canonical · [`zask.rs`](zask.rs) cross-shell prompts |
| **Ingest** | [`fsnotify.rs`](fsnotify.rs) debounced watcher · [`source_resolver.rs`](source_resolver.rs) · [`export.rs`](export.rs) view/export/import · [`metrics.rs`](metrics.rs) |
| **Builtin shims** | [`zask_builtin.rs`](zask_builtin.rs) · [`zcomplete_builtin.rs`](zcomplete_builtin.rs) · [`zhistory_builtin.rs`](zhistory_builtin.rs) · [`zjob_builtin.rs`](zjob_builtin.rs) · [`zsource_builtin.rs`](zsource_builtin.rs) · [`zsync_builtin.rs`](zsync_builtin.rs) |

`zd_dispatch.rs` is shared by two callers over different transports: the `zd`
binary (HTTP) and the `zd` shell builtin (Unix socket, in-process, no fork).

---

## [0x03] ON-DISK LAYOUT

One root directory, `0700`, holding everything. `$ZSHRS_HOME` overrides;
default `~/.zshrs/` ([`paths.rs`](paths.rs) `resolve` / `with_root`).

```
~/.zshrs/
├── daemon.sock              unix socket
├── daemon.pid               singleton flock target
├── index.rkyv               shard index
├── images/                  rkyv shard images (mmap, zero-copy)
├── catalog.db               SQLite mirror — catalog
├── history.db               SQLite mirror — history
├── cache.db                 SQLite mirror — kv cache
├── artifacts/               content-addressed blobs
├── snapshots/               saved canonical states
├── replay/                  replay logs
├── zshrs-daemon.log         tracing output (rolling)
├── zshrs-daemon.toml        daemon knobs
├── zshrs.toml               shell-side knobs
└── zshrs-recorder.toml      recorder knobs
```

Config files are seeded with defaults on first run and never overwrite user
edits. Databases and configs are forced to `0600`, directories to `0700`, on
every startup.

---

## [0x04] IPC PROTOCOL

```
┌──────────────────────┬────────────────────────────────┐
│ u32 big-endian length│ UTF-8 JSON body                │
└──────────────────────┴────────────────────────────────┘
```

| Property | Value |
|----------|-------|
| Version | `PROTOCOL_VERSION = 1` ([`ipc.rs:25`](ipc.rs)) |
| Max frame | 64 MiB (`MAX_FRAME_BYTES`) — larger is rejected, not truncated |
| Handshake | `Hello { version }` → `Welcome`; mismatch closes with `ProtocolMismatch` |
| Framing | length-prefixed JSON, one op per frame |

Errors are typed (`DaemonError` in [`lib.rs`](lib.rs)): protocol mismatch,
malformed handshake, oversized frame, unknown opcode, bad args, timeout.

---

## [0x05] OP SURFACE

120+ ops, dispatched by string opcode in [`ops.rs`](ops.rs). Unknown opcodes
return `UnknownOp`.

| Group | Ops |
|-------|-----|
| Session / registry | `info` `ping` `list_shells` `tag` `untag` `send` `cmd_result` `cmd_started` `notify` `register` `doctor` |
| Daemon control | `daemon` (status / stop / restart) `config_get` `config_set` `config_list` `metrics` |
| Cache | `cache_put` `cache_get` `cache_del` `cache_list` `cache_stats` |
| Catalog / canonical | `recorder_ingest` `canonical_hydrate_view` `clean` `verify` `compact` `source_resolve` |
| History | `history_append` `history_query` |
| Pub/sub | `subscribe` `unsubscribe` `subscription_set_paused` `publish` `subscribe_shard` |
| Watch | `watch_subscribe` `watch_unsubscribe` `watch_list` `fpath_changed` `watcher_stats` |
| Sync | `push_canonical` `pull_canonical` `diff_canonical` |
| Export / import | `view` `export` `export_all` `export_catalog` `export_shard` `export_zcompdump` `import_*` `replay_log` |
| Jobs | `job_submit` `job_list` `job_status` `job_output` `job_kill` `job_cancel` `job_wait` `job_input` `job_resize` |
| Locks | `lock_acquire` `lock_try_acquire` `lock_release` `lock_list` |
| Artifacts | `artifact_put` `artifact_get` `artifact_get_by_digest` `artifact_list` `artifact_gc` |
| Snapshots | `snapshot_save` `snapshot_list` `snapshot_load` `snapshot_diff` |
| Schedule | `schedule_add` `schedule_add_once` `schedule_remove` `schedule_list` |
| Definitions | `definitions_query` `definitions_kinds` `definitions_emit` `definitions_diff` `definitions_subscribe` `definitions_unsubscribe` |
| Ask (cross-shell prompts) | `ask_ask` `ask_pending` `ask_take` `ask_dismiss` `ask_response` |
| Completion | `complete` `suggest` `highlight` |
| Logging | `log_level` `log_rotate` `log_stats` |

Shell-side, these are reached through z\* builtins — `zcache`, `zls`, `zid`,
`zping`, `ztag`, `zuntag`, `zsend`, `znotify`, `zsubscribe`, `zunsubscribe`,
`zjob`, `zsync`, `zask` ([`builtins.rs`](builtins.rs)). Names owned by zsh
(`zmv`, `zparseopts`, `zstyle`, `zcompile`, …) are never shadowed.

---

## [0x06] HTTP SURFACE

Off by default. Set `[http] listen` in `zshrs-daemon.toml` to enable. See
[`docs/DAEMON_AS_SERVICE.md`](../docs/DAEMON_AS_SERVICE.md) and
[`examples/daemon-curl-cookbook.md`](../examples/daemon-curl-cookbook.md).

| Route | Purpose |
|-------|---------|
| `GET /health` | liveness |
| `GET /metrics` | daemon metrics |
| `POST /op/:name` | invoke any IPC op over HTTP |
| `GET /ops` | op catalog |
| `GET /openapi` · `/openapi.json` | generated spec |
| `GET /stream/events` | SSE — daemon events |
| `GET /stream/definitions` | SSE — definition changes |
| `GET /stream/watch` | SSE — watch notifications |

**Auth model** ([`auth.rs`](auth.rs), [`http.rs`](http.rs)):

- No tokens + loopback bind → open access (the single-user default).
- No tokens + **non-loopback** bind → **refused at startup.** The daemon will
  not expose itself off-host unauthenticated.
- Tokens configured → every request needs `Authorization: Bearer <token>`.

Tokens are flat strings (full access, back-compatible) or scoped tables with
`<area>.<verb>` scopes (`cache.put`, `defs.read`, `meta.admin`; wildcards `*`,
`cache.*`, `*.read`). Ops absent from the `op_scope()` table default to
`meta.admin` — deny-by-default for any newly added op.

Client shims for eight shells ship in
[`examples/`](../examples/): `daemon-shell.{sh,bash,zsh,ksh,fish,elv,nu,ps1}`.

---

## [0x07] BINARIES

| Binary | Purpose |
|--------|---------|
| `zshrs-daemon` | standalone daemon entrypoint — systemd/launchd, CI cache servers, debugging under a distinct process name |
| `zshrs-daemon-bench` | daemon benchmark driver |

The shell normally spawns the daemon itself (`zshrs --daemon`); this binary is
the same `run()` packaged separately.

```
zshrs-daemon [OPTIONS]

  --home <DIR>            override $ZSHRS_HOME (default ~/.zshrs/)
  --log-level <DIRECTIVE> override ZSHRS_LOG (info | debug | info,fsnotify=trace)
  --log-stderr            also stream tracing to stderr
  --verbose-init          show daemon work on every run (implies --log-stderr, debug)
  --quiet-first-run       suppress the first-run stderr block
  --print-paths           print resolved paths as JSON, exit
  --check-config          validate config as JSON, exit
  --version / -h, --help
```

`--print-paths` and `--check-config` let editors and CI pre-flight a config
edit without restarting the daemon. Exits cleanly on SIGTERM, SIGINT, or
`zcache daemon stop`. Completion: [`completions/_zshrs-daemon`](../completions/_zshrs-daemon).

---

## [0x08] BUILD & TEST

Workspace member of the [zshrs](https://github.com/MenkeTechnologies/zshrs) root
crate (`members = [".", "daemon"]`).

```sh
cargo build -p zshrs-daemon
cargo test  -p zshrs-daemon
```

Integration tests live in the parent crate:
[`tests/daemon_integration.rs`](../tests/daemon_integration.rs),
[`tests/daemon_http.rs`](../tests/daemon_http.rs).

The parent's `daemon` feature is on by default and gates the dependency:

```sh
cargo build --no-default-features   # zshrs lib without the daemon crate
```

Dependencies are held to the durability bar in the project's design goals —
tokio, axum/hyper/tower, rusqlite, rkyv, nix, notify, serde. No build-time
network fetches, no bit-rotting install scripts.

---

## [0xFF] LICENSE

MIT — Part of the [zshrs](https://github.com/MenkeTechnologies/zshrs) project. Copyright (c) 2026 [MenkeTechnologies](https://github.com/MenkeTechnologies)
