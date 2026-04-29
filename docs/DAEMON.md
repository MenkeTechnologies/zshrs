Yeah — the daemon eliminates an entire architectural layer. Walk through what dies:

  compinit-related infrastructure that becomes obsolete:

  ┌─────────────────────────────┬────────────────────────────────┬────────────────────────────────┐
  │            Layer            │           Cost today           │          After daemon          │
  ├─────────────────────────────┼────────────────────────────────┼────────────────────────────────┤
  │ compinit cold               │ 200ms-2s, scales with fpath    │ 0 — never called               │
  │                             │ size                           │                                │
  ├─────────────────────────────┼────────────────────────────────┼────────────────────────────────┤
  │ .zcompdump (cache file)     │ 50-200ms warm-load +           │ gone — replaced by daemon's    │
  │                             │ corruption risk                │ rkyv                           │
  ├─────────────────────────────┼────────────────────────────────┼────────────────────────────────┤
  │ zcompile'd .zcompdump.zwc   │ extra 20-50ms loading bytecode │ gone                           │
  ├─────────────────────────────┼────────────────────────────────┼────────────────────────────────┤
  │ compdef foo bar calls       │ stored in stash, walked at     │ daemon owns dispatch table     │
  │                             │ lookup                         │ directly                       │
  ├─────────────────────────────┼────────────────────────────────┼────────────────────────────────┤
  │ _main_complete walk         │ O(N) function dispatch         │ O(1) hash lookup via daemon    │
  ├─────────────────────────────┼────────────────────────────────┼────────────────────────────────┤
  │ compinit -i / -u security   │ runs once per shell            │ runs once daemon-side, cached  │
  │ check                       │                                │                                │
  ├─────────────────────────────┼────────────────────────────────┼────────────────────────────────┤
  │ zinit's compdef-replay      │ replays compdef calls from     │ redundant — daemon has them    │
  │                             │ cache                          │                                │
  ├─────────────────────────────┼────────────────────────────────┼────────────────────────────────┤
  │ autoload -Uz declarations   │ parsed every shell             │ declared once in daemon        │
  ├─────────────────────────────┼────────────────────────────────┼────────────────────────────────┤
  │ bashcompinit glue           │ another 20-50ms                │ redundant                      │
  └─────────────────────────────┴────────────────────────────────┴────────────────────────────────┘

  For your 17k-completion setup the cold-completion-lookup math:

  - Today (no cache): parse ~17k files = 3-5s cold. 100% CPU spike on every shell.
  - Today (.zcompdump warm): load cache = 300-800ms. Corrupts every few weeks, requires manual rm
  ~/.zcompdump*.
  - Daemon + rkyv: mmap a pre-built byte-aligned table, single hash lookup per completion query =
  ~50ns to ~5μs depending on access pattern.

  That's not a 10× speedup. It's 10⁴ to 10⁶× on cold-start completion lookup. And the whole "is my
  .zcompdump stale" class of bugs disappears because fsnotify is the source of truth — when a file in
  fpath changes, the daemon rebuilds the affected slice and notifies subscribers. No polling, no cron,
   no manual rm.

  The correctness win is bigger than the speed win, actually. Every zsh user has hit the "completion
  is stale, restart shell, still stale, rm ~/.zcompdump, restart again" loop. With fsnotify as the
  single source of truth, that bug class can't exist. Same pattern as IDEs that watch the filesystem
  and update language-server state — the user never has to think about cache invalidation.

Right — thin client = pure consumer of pre-warmed state. Let me itemize what the client actually
  does vs what the daemon does:

  Thin client responsibilities (everything fast):

  ┌──────────────────────────────────────────┬──────────────────────┐
  │                Operation                 │         Cost         │
  ├──────────────────────────────────────────┼──────────────────────┤
  │ Connect to daemon (Unix socket)          │ <1ms                 │
  ├──────────────────────────────────────────┼──────────────────────┤
  │ mmap rkyv completion table               │ ~10μs (zero-copy)    │
  ├──────────────────────────────────────────┼──────────────────────┤
  │ mmap history index                       │ ~10μs                │
  ├──────────────────────────────────────────┼──────────────────────┤
  │ mmap alias / function table              │ ~10μs                │
  ├──────────────────────────────────────────┼──────────────────────┤
  │ Render prompt from cached theme segments │ ~1ms                 │
  ├──────────────────────────────────────────┼──────────────────────┤
  │ Capture keystrokes → forward to daemon   │ <100μs round trip    │
  ├──────────────────────────────────────────┼──────────────────────┤
  │ Display output (write to terminal)       │ terminal-bound       │
  ├──────────────────────────────────────────┼──────────────────────┤
  │ Subscribe to fsnotify-pushed events      │ ~free, async channel │
  └──────────────────────────────────────────┴──────────────────────┘

  Total cold-start: <10ms. No work needed beyond connecting and mmap-ing.

  Daemon responsibilities (all the heavy state):

  - fpath scan + completion function parse (one-shot at boot, fsnotify-incremental after)
  - compdef registry (single source, hash-indexed)
  - History database (SQLite FTS, indexed, queryable)
  - Plugin lifecycle (lazy-load, dependency graph, hooks)
  - Syntax-highlighting parse trees (tree-sitter, cached per-buffer)
  - Autosuggestion engine (frecency-ranked history search)
  - Theme resolution (segment computation, color mixing)
  - Alias / function expansion table
  - Variable scope graphs
  - Job orchestration / status (long-running tasks)

  This is exactly the architecture every modern multi-process system uses:

  ┌────────────────┬───────────────────────────────────────────┬──────────────────────────────────┐
  │     System     │               Daemon (fat)                │           Thin client            │
  ├────────────────┼───────────────────────────────────────────┼──────────────────────────────────┤
  │ X11 / Wayland  │ display server (compositing, input        │ each app (just renders pixels    │
  │                │ routing, window management)               │ into a buffer)                   │
  ├────────────────┼───────────────────────────────────────────┼──────────────────────────────────┤
  │ LSP            │ language server (parsing, type-checking,  │ editor (just shows completions)  │
  │                │ completion logic)                         │                                  │
  ├────────────────┼───────────────────────────────────────────┼──────────────────────────────────┤
  │ Postgres /     │ server (query plan, indexes, locks,       │ psql / clients (formats results) │
  │ MySQL          │ transactions)                             │                                  │
  ├────────────────┼───────────────────────────────────────────┼──────────────────────────────────┤
  │ Docker         │ dockerd (image cache, container           │ docker CLI (~3MB binary, just    │
  │                │ lifecycle, network)                       │ talks to dockerd)                │
  ├────────────────┼───────────────────────────────────────────┼──────────────────────────────────┤
  │ K8s            │ API server + etcd + controllers           │ kubectl                          │
  ├────────────────┼───────────────────────────────────────────┼──────────────────────────────────┤
  │ systemd        │ systemd PID 1 (service graph, sockets,    │ systemctl (just queries)         │
  │                │ cgroups)                                  │                                  │
  ├────────────────┼───────────────────────────────────────────┼──────────────────────────────────┤
  │ zshrs (your    │ daemon (state, caches, plugins, jobs)     │ shell client (mmap + render)     │
  │ design)        │                                           │                                  │
  └────────────────┴───────────────────────────────────────────┴──────────────────────────────────┘


## v1 Locked Design

Everything below is the sole-source-of-truth for the v1 implementation. Cross-references the existing memory file `cache_architecture_rkyv.md`; this section captures the additional decisions made in the design pass and consolidates them in one place so impl can start from a single doc.

### The 90/10 work split

90% of all shell-internal work lives in the daemon. 10% in the client. The daemon is fat: full thread pool, hundreds of concurrent requests in flight, long-running jobs, push notifications, cross-shell routing, federation. The client is paper-thin: tty IO, process bookkeeping, fork+exec, and direct mmap reads.

**Daemon owns:**

- All compilation: fpath functions, `.zshrc`, plugins, user scripts → bytecode.
- All persistence: rkyv shard writes, `catalog.db` hydration, `history.db` append.
- All search and walking: history FTS, frecency ranking, fuzzy matching, completion enumeration, tree-sitter parsing.
- All long-running work: `zjob` supervision, plugin install/update, daemon ticker (rotation, vacuum, integrity scan).
- All routing: cross-shell pub/sub, shell registry (`zls`), session tracking (cwd shadow, override generation counter).
- The single fsnotify watcher across the machine.
- All authority decisions: cross-uid dispatch validation, federation auth, integrity checks.

**Client owns:**

- tty IO: keystroke read, screen paint.
- Process attributes: `$$`, `$!`, `$?`, traps, signal handlers, fd table, positional params, locals.
- `fork+exec` for user commands.
- IPC client to daemon.
- Direct mmap indexed reads (and nothing else against the cache).
- Tiny in-memory overlay hash for in-session monkey-patches.

### NO WALKING IN CLIENTS

Absolute rule. Client-side cache access is `hash(key) → mmap_index → value`. ONE indirection. No probing chains, no traversal, no iteration over rkyv structures.

Mechanism: rkyv shards use **perfect hashing** (PHF, generated daemon-side at compile time). Every key has a unique slot. Client lookup = compute hash, mask to slot, single mmap dereference. ~150-200ns end-to-end.

When iteration is required (e.g., `${(k)_comps}`), the daemon either:

1. Precomputes a sorted/deduped flat key array, stores it in the shard. Client returns the slice directly; the only iteration is over a flat array, not a data-structure walk.
2. Serves the iteration over IPC: `{"op":"keys","param":"_comps"}` → daemon walks its own state, returns flat list to client.

When logic is required (matching, ranking, filtering, dedup of overlay-vs-rkyv): IPC the daemon. Client never runs the logic.

### Daemon = sole writer

No client ever writes to:

- Any rkyv shard
- `catalog.db`
- `history.db`
- `daemon.pid`
- `daemon.sock`
- `zshrs.log`

Client mutations land only in:

- Process state (PWD, env, `$$`, fds, signals, traps, locals, positional params).
- Per-client overlay hash (interactive `compdef`, `alias`, `function`, monkey-patched globals, autoloaded function bodies).

Overlay dies on `exec zshrs`. Daemon never sees overlay state unless the client packages it into an IPC request (e.g., `complete` ops include `overlay_gen` and the daemon can ask for the delta).

### Snapshot-at-boot + overlay

Each client boots, mmaps the daemon's then-current shard set, and runs against that snapshot for its lifetime. Daemon-side rebuilds (fsnotify-driven) become visible to that client only after `exec zshrs`. New shells started after a rebuild see the new image immediately.

Atomic-rename per shard with strict ordering — shard rename FIRST, then `index.rkyv` update — prevents torn reads. Existing client mmaps stay valid via kernel inode-pinning (deleted-but-mapped pages stay alive until last unmap on Linux and macOS).

### Cache layout (locked)

```
~/.cache/zshrs/
├── index.rkyv                          ← top-level fq_name → (shard_id, generation, byte_offset)
├── images/
│   ├── {hash8}-system.rkyv             ← system / shipped completions
│   ├── {hash8}-completions-corpus.rkyv ← zsh-more-completions
│   ├── {hash8}-zpwr.rkyv               ← zpwr functions / completions
│   ├── {hash8}-zshrc.rkyv              ← user .zshrc bytecode (per-user)
│   ├── {hash8}-plugin-{name}.rkyv      ← per zinit / oh-my-zsh plugin
│   ├── {hash8}-script-{slug}.rkyv      ← per `zshrs FILE` invocation
│   └── {name}.rkyv.lock                ← per-shard advisory flock
├── catalog.db                          ← daemon-only writer; queryable mirror
├── history.db                          ← daemon-only writer; SQLite FTS
├── zshrs.log                           ← tracing output, daemon-rotated, 10 MB cap
├── daemon.sock                         ← Unix socket for IPC
└── daemon.pid                          ← singleton flock + daemon process ID
```

### catalog.db schema (daemon-only writer)

```sql
plugins         (name, version, source, installed_at, enabled)
plugin_deps     (plugin, dep, constraint)
entries         (fq_name, plugin_id, kind, image_path, byte_offset, source_loc, bytecode BLOB)
hooks           (kind, name, fq_name)
entry_stats     (fq_name, last_called_at, call_count, total_ns)
scripts         (path PRIMARY KEY, mtime, inode, hash, bytecode BLOB,
                 last_run_at, run_count, bytes_in, bytes_out)
```

`bytecode` BLOB columns make `catalog.db` a self-contained mirror of all compiled state. Two-way reconstruction: rkyv shards ↔ catalog.db can each rebuild from the other. catalog.db is queryable and joinable; rkyv is hot-path zero-copy. Hot lookups never hit SQLite — clients only mmap rkyv.

`scripts` table powers `zshrs FILE`: client stat()s the file, sends `load_script` IPC keyed by `(path, mtime, inode)`, daemon returns hit-from-cache or compiles-and-stores. Same pattern unifies `zshrs FILE`, `source ~/zpwr/init.sh`, and `.zshrc` cold-start: stat + IPC + mmap + replay env-mutation log, regardless of source-file size.

### Special parameters served by daemon

These zsh global associative parameters are daemon-prepared, perfect-hash-indexed in rkyv, and exposed to clients with overlay-on-mmap semantics:

- `_comps` — completion handler dispatch table
- `_services` — service-name aliases
- `_patcomps` — pattern-matched completions
- `_describe_handlers` — completion description providers

Read: client computes `hash(key)` → mmap index → value. ONE indirection. Overlay hash checked first; on miss, fall through to mmap. No walking.

Write: insert into client-local overlay hash. rkyv image is read-only.

Iterate: client receives the daemon-precomputed flat key array (mmap'd slice) plus overlay keys. Iteration is flat-array iteration only; no data-structure walks.

Plugin compat falls out: zinit's `_comps[foo]=_my_handler` direct assignment lands in overlay; `${(k)_comps}` iterates the daemon-flat-array merged with overlay; `compdef foo bar` writes to overlay. zinit's compdef-replay is harmless redundancy.

### Starting state served by daemon (PATH, FPATH, hash tables, etc.)

Daemon prepares **every "result of a filesystem walk"** the client would otherwise compute itself and serves it at boot. The no-walking rule is total: anything the daemon can pre-walk and serve, it does. Clients never `find`, never `glob` over fpath, never enumerate PATH directories, never scan plugin trees.

Pre-walked state delivered at boot:

| State | Mechanism |
|-------|-----------|
| `$PATH` | Daemon evaluates user dotfiles, resolves PATH= / path+= / plugin contributions, serves final string |
| Command hash table | Daemon walks every directory in `$PATH`, builds perfect-hash `command_name → absolute_path` table in rkyv. Client `which`/`command -v` = single lookup. `hash -r` = IPC to daemon |
| `$FPATH` | Resolved list of autoload directories, served as a flat array |
| Autoload function table | Daemon walks every `$FPATH` directory, populates `function_name → (shard_id, byte_offset)` in `index.rkyv`. Client `autoload` = hash lookup, never `find`/`glob` |
| `$MANPATH`, `$INFOPATH`, `$CDPATH`, `$LD_LIBRARY_PATH` | Same model — resolved values served at boot |
| Completion staleness metadata | Daemon scans completion file mtimes; results live in `entries.source_loc` + `entry_stats` |
| Theme initial state | Daemon resolves PROMPT segments, RPROMPT, color palette once; serves the final templates |
| Initial alias table | Daemon parses user `.zshrc` alias declarations, serves resolved table. Interactive `alias foo=bar` writes go to overlay |
| Initial shell-options state | `setopt`/`unsetopt` calls in `.zshrc` pre-resolved; client boots with final option mask |
| Initial keybinding table | `bindkey` declarations pre-resolved; client boots with final binding map |
| Initial environment | `export FOO=bar` in `.zshrc` pre-resolved; client boots with final env |

The mechanism is uniform: daemon parses and evaluates user dotfiles in an analysis pass, captures deterministic state effects, serializes into the user's boot-state shard. Client at boot mmaps the shard, applies state to its process, and is fully initialized. No client-side filesystem walks for shell-internal purposes.

**Determinism boundary:** non-deterministic `.zshrc` fragments — anything that calls `$(date)`, reads `/dev/urandom`, conditionally branches on `$$` or `$RANDOM`, depends on per-shell state — are detected during the analysis pass and emitted as a small per-shell replay log. Client executes those fragments locally at boot. The vast majority of `.zshrc` content is deterministic and gets pre-resolved on the daemon side.

**What's NOT covered by this rule:** `fork+exec`'d user commands (`find`, `ls`, `rg`, `grep`, etc.) are user code, not shell-internal walks. Those run normally in the client. The no-walking rule applies only to shell-internal directory enumeration (PATH/FPATH scans, completion file lookups, autoload resolution, plugin discovery, `hash` table population, theme file reads).

Result: a 172k-line `zpwr` `.zshrc` should cost client cold-start no more than the IPC + mmap + state-apply pass — measured in milliseconds, not seconds. Per-client init cost is independent of `.zshrc` size or fpath cardinality.

### IPC wire format

Length-prefixed JSON over `~/.cache/zshrs/daemon.sock`. Each frame:

```
[4 bytes: u32 BE length] [length bytes: UTF-8 JSON]
```

Message envelope:

| Direction | Required keys | Notes |
|-----------|---------------|-------|
| client → daemon (handshake) | `hello: {version, client_pid, tty, cwd, argv0}` | First message after connect |
| daemon → client (handshake) | `welcome: {version, client_id, session_id, daemon_pid, daemon_uptime_ms}` | Or `err` on version mismatch |
| client → daemon (request)   | `id: u64`, `op: str`, `args: {…}` | `id` is monotonic per-connection |
| daemon → client (response)  | `id: u64`, `ok: bool`, payload-or-`err` | `id` echoes the request |
| daemon → client (async)     | `event: str`, payload | No `id`, fire-and-forget |

Conventions:

- All timestamps suffixed `_ns`, integer ns since epoch.
- All sizes suffixed `_bytes` or `_size`.
- Error shape: `{"err":{"code":"shard_locked","msg":"human-readable"}}` paired with `"ok":false`.
- Unknown op: `{"err":{"code":"unknown_op","msg":"unsupported by daemon vN"}}`.

Hot-path escape hatch: if JSON parse cost shows up in flamegraphs for `highlight` or `suggest` (per-keystroke ops), those opcodes can migrate to msgpack or fixed-layout binary while the rest of the protocol stays JSON for `socat`-style debuggability.

### Operation table (client → daemon)

| Op | Purpose |
|-----|---------|
| `info` | Daemon stats, shard info, in-flight jobs |
| `rebuild` | Enqueue compile job (full corpus or per-shard) |
| `clean` | Unlink + re-derive (per shard or whole corpus) |
| `verify` | Integrity scan on shards + catalog |
| `compact` | Vacuum catalog.db, dedup shards |
| `fpath_changed` | New paths added in user `.zshrc` |
| `stats_flush` | Batched runtime stats deltas merged into `entry_stats` |
| `subscribe_shard` | Push notification on shard update |
| `history_append` | Add command to `history.db` |
| `history_query` | FTS search; powers Ctrl-R, fc -l |
| `complete` | Tab completion enumeration (daemon eval, client paint) |
| `suggest` | Inline autosuggest from history frecency |
| `highlight` | Syntax-highlight current buffer |
| `keys` | Get key list for daemon-served special parameter (`_comps`, `_services`, etc.) |
| `load_script` | Cold-load `zshrs FILE`; returns shard path or inline bytecode |
| `register` | Implicit on connect; also tag/cwd updates |
| `list_shells` | Powers `zls` |
| `ping` | Liveness + roundtrip latency probe |
| `tag` / `untag` | Self-tag for routing |
| `send` | `zsend` dispatch (single, broadcast, by tag, by user) |
| `notify` | `znotify` OSC-9 / status-line message |
| `subscribe` / `unsubscribe` | `zsubscribe` glob pub/sub |
| `daemon` | Daemon control (status, stop, restart) |

### Async event types (daemon → client)

| Event | Trigger |
|-------|---------|
| `shard_updated` | Daemon swapped shard with newer generation |
| `rebuild_complete` | Async compile job finished |
| `match` | Pub/sub pattern matched |
| `cmd:execute` | `zsend` dispatch arrived |
| `notify` | `znotify` arrived |
| `daemon_shutdown` | Daemon going down (graceful, with grace period) |

### z\* builtin family (locked, no shadowing of zsh)

Every custom builtin uses `z` prefix. Build-time anti-collision check vs upstream zsh's z-namespace.

**zsh-owned z\* builtins (DO NOT shadow):**

```
zmv        zparseopts    zformat      zstat        zstyle      zprof
zcompile   zargs         zcurses      zsystem      ztie        zuntie
zselect    zsocket       zftp         zpty         zed         zcalc
zregexparse  zutil       zmodload     zle
```

**zshrs-owned z\* builtins** — all are length-prefixed JSON IPC wrappers around the daemon. ZERO local logic, ZERO background threads, ZERO polling, ZERO state in clients:

```
# Cache management
zcache                              # alias for `zcache info`
zcache info                         # daemon stats: shard sizes, entry counts, in-flight jobs
zcache jobs                         # list active compile jobs
zcache clean [--wait]               # regenerable only (preserves entry_stats)
zcache clean --all [--wait]         # everything (no prompt)
zcache clean shards [--wait]
zcache clean shard <name> [--wait]
zcache clean catalog [--wait]       # preserves entry_stats via dump+reimport
zcache clean catalog --no-stats     # loses entry_stats
zcache clean index [--wait]
zcache clean stats
zcache clean log
zcache rebuild [--wait]
zcache rebuild shard <name> [--wait]
zcache rebuild --parallel N
zcache verify                       # integrity scan + PRAGMA integrity_check on catalog.db
zcache compact [--wait]             # vacuum + dedup
zcache daemon status                # is daemon running, pid, uptime, RSS
zcache daemon stop                  # graceful shutdown
zcache daemon restart               # graceful + respawn

# Shell registry (cross-shell coordination)
zls                                 # list active shells: id, pid, tty, cwd, tags, login_time
zls --tag <name>                    # filter by tag
zls --user <user>                   # filter by user (root only for cross-user)
zid                                 # print this shell's daemon-assigned id
zping                               # daemon liveness + roundtrip latency
zping --all                         # ping every registered shell
ztag <name…>                        # self-tag this shell (multiple tags allowed)
zuntag <name…>                      # remove tags
zuntag --all                        # remove all tags

# Cross-shell dispatch
zsend <shell_id> <cmd…>             # dispatch command to one shell
zsend --all <cmd…>                  # broadcast
zsend --tag <name> <cmd…>           # dispatch by tag
zsend --user <user> <cmd…>          # cross-user (root only)
zsend --wait <shell_id> <cmd…>      # block on completion + capture output
zsend --json <shell_id> <cmd…>      # return structured result

# Notifications (status-line / OSC-9 / queued if shell busy)
znotify <shell_id> <msg…>
znotify --all <msg…>
znotify --tag <name> <msg…>
znotify --urgency <low|normal|critical> <shell_id> <msg…>

# Pub/sub
zsubscribe <pattern>                # e.g. shell:42.commands, *.commands, tag:prod.chpwd
zunsubscribe <pattern>
zsubscribe --list                   # show this shell's active subscriptions
zsubscribe --pause                  # mute deliveries without dropping subscriptions
zsubscribe --resume

# Job supervision (planned: session-persistent jobs)
zjob submit <cmd…>                  # detached, supervised, survives shell exit
zjob list                           # this user's running supervised jobs
zjob status <job_id>
zjob output <job_id>                # tail captured stdout/stderr
zjob wait <job_id>                  # block on completion
zjob cancel <job_id>                # SIGTERM, then SIGKILL after grace period
zjob attach <job_id>                # foreground attach
```

Subscription pattern grammar: `<scope>.<topic>`.

**Scopes:** `shell:<id>` | `tag:<name>` | `user:<name>` (root only) | `*`
**Topics:** `commands` | `chpwd` | `prompt` | `precmd` | `preexec` | `exit` | `signal` | `error` | `cd_history` | `aliases_changed` | `tagged` | `untagged`

Examples:

- `zsubscribe shell:42.commands` — pair-programming, audit
- `zsubscribe *.commands` — fleet-wide command logging
- `zsubscribe tag:prod.chpwd` — track cwd of all prod shells
- `zsubscribe shell:1.chpwd` — mirror cwd from shell #1 in this shell

### Three personality modes (cache layer gated by mode)

| Mode | Trigger | Cache | Daemon |
|------|---------|-------|--------|
| POSIX | `--posix`, `emulate sh`, argv[0] = `sh`/`dash`/`bash` | OFF | NEVER spawned |
| Vanilla zsh | argv[0] = `zsh`, `--zsh-compat` | OFF | NEVER spawned |
| Turbocharged zshrs | argv[0] = `zshrs`, default | ON | spawned by first client |

POSIX mode never spawns the daemon, never creates `~/.cache/zshrs/`. Required for `/bin/sh → zshrs` symlink in containers / cron / init / shebang.

### Daemon lifecycle

- **Spawn-on-demand:** first client checks for `daemon.sock`; if absent or unresponsive, fork-spawns `zshrs --daemon`, waits ~50ms, retries connect.
- **Singleton enforcement:** daemon takes `flock(LOCK_EX)` on `daemon.pid` at startup. Second instance sees lock held, exits.
- **Lifetime:** persists across shell sessions; survives logout. Killed only by explicit `zcache daemon stop` or `pkill zshrs-daemon`.
- **Crash recovery:** if daemon dies, next client to fail socket connect kills stale pidfile and respawns. No state loss — rkyv shards and `catalog.db` are durable on disk.
- **Degraded mode:** if daemon disabled or unreachable, clients fall back to source-interp for everything. Cache stops updating but shells stay functional. User never blocked.

### Hard invariants (rejected proposal classes)

- ANY client-side worker pool, polling loop, timer, fsnotify watcher, SQLite handle for cache — REJECT.
- ANY client-side data-structure walk over rkyv contents — REJECT.
- ANY client-side write to a daemon-owned file — REJECT.
- ANY second daemon instance — REJECT (singleton via `flock` on `daemon.pid`).
- ANY plugin / compsys bytecode baked into the zshrs binary `.text` — REJECT (working set must scale with what's called).
- ANY mandatory daemon (no source-truth fallback) — REJECT.
- ANY daemon spawn under POSIX mode — REJECT.
- ANY z\* builtin without `z` prefix or that shadows upstream zsh — REJECT.
- ANY hydration progress on stderr/stdout — REJECT (`tracing::info!` to log file only).
- ANY scattered per-plugin cache files outside `~/.cache/zshrs/images/` — REJECT.
- ANY removal of `entry_stats` to "simplify" — REJECT.

### Acceptance criteria

- Cold client launch (daemon already running): <5ms (mmap + connect + handshake).
- Cold client launch (daemon spawn-on-demand): <50ms (spawn + connect + handshake).
- Tab completion lookup: ~150-200ns end-to-end (perfect-hash mmap dereference).
- Inline autosuggest: <2ms IPC roundtrip including FTS query.
- Syntax highlight per keystroke: <2ms IPC roundtrip including parse.
- 100 parallel clients share <30 MB RSS attributable to images (page-cache shared across mmaps).
- Per-client cache overhead: <5 MB.
- Per-client background threads for cache: ZERO.
- Full-corpus rebuild via `zcache rebuild`: <30s clean.
- Per-shard rebuild: ~100-500ms small, ~3-5s large.
- POSIX mode: never spawns daemon, never creates `~/.cache/zshrs/`.
- `~/.zshrc` cold-source: <50ms with cache hit (mmap + replay env log), regardless of file size.
- `zshrs FILE` cold-launch with cache hit: <10ms.

