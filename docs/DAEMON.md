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
- **All filesystem enumeration:** `$PATH` dir scans, `$FPATH` dir scans, plugin tree walks, completion file discovery, theme file reads, all `find`/`glob`/`readdir` over shell-internal directories.
- **All starting-state preparation:** `$PATH`/`$FPATH`/`$MANPATH` resolution, command hash table, autoload function table, alias table, shell-options state, keybindings, theme templates — all pre-resolved daemon-side and served at boot.
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

Absolute rule. Two surfaces this covers:

**1. No client-side data-structure traversal.** Client-side cache access is `hash(key) → mmap_index → value`. ONE indirection. No probing chains, no traversal, no iteration over rkyv internal structures. Mechanism: rkyv shards use **perfect hashing** (PHF, generated daemon-side at compile time). Every key has a unique slot. Client lookup = compute hash, mask to slot, single mmap dereference. ~150-200ns end-to-end.

**2. No client-side filesystem walks for shell-internal purposes.** Clients never `find`, `glob`, `readdir`, or stat-loop over `$PATH`, `$FPATH`, plugin trees, completion directories, or any other shell-internal source. The daemon walks everything once and serves the precomputed results (see "Starting state served by daemon" below). Even `hash -r` becomes an IPC to the daemon, not a client-side rebuild of the command hash table.

When iteration is required (e.g., `${(k)_comps}`, `for cmd in $(hash); do …`), the daemon either:

1. Precomputes a sorted/deduped flat key array, stores it in the shard. Client receives the slice directly; the only iteration is over a flat array, not a data-structure walk.
2. Serves the iteration over IPC: `{"op":"keys","param":"_comps"}` → daemon walks its own state, returns flat list to client.

When logic is required (matching, ranking, filtering, dedup of overlay-vs-rkyv): IPC the daemon. Client never runs the logic.

Exception (intentional): `fork+exec`'d user commands (`find`, `ls`, `rg`, `grep`) walk filesystems normally — those are user code, not shell-internal walks.

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
~/.zshrs/
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
compiled_files  (path PRIMARY KEY, kind TEXT, mtime, inode, hash, bytecode BLOB,
                 last_used_at, use_count, bytes_in, bytes_out, sensitive BOOLEAN,
                 parent_paths TEXT)
                 -- kind ∈ {'script', 'source', 'zshrc', 'plugin_init', 'autoload'}
                 -- one unified table for any file that gets parsed and bytecoded by the daemon,
                 -- looked up by absolute path. covers `zshrs FILE`, `source FILE`, .zshrc,
                 -- plugin entry-point files, autoloaded function files. parent_paths is a
                 -- JSON array of files that transitively included this one (for invalidation
                 -- chain when an upstream source dependency changes).
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

For legacy tooling that introspects `.zcompdump` directly (some plugin patterns, backup scripts, p10k cache-staleness probes, parallel zsh sessions sharing the cache), the daemon can synthesize a valid `.zcompdump` file on demand from its canonical state. Triggered by `zcache export zcompdump [path]` or the `export_zcompdump` IPC op. The synthesized file is byte-compatible with what `compinit` would have produced, so legacy consumers don't notice the difference. Not generated automatically — opt-in only, on user request. (`.zcompdump.zwc` is not emitted; legacy tooling will regenerate it if it wants it.)

### Starting state captured by recorder, stored by daemon

**Architecture pivot from v0 spec:** the original design had the daemon
parse `.zshrc` + every plugin in a multi-pass analysis pass and derive
starting state automatically. That code was deleted. State attribution
now flows through `zshrs-recorder` — a separate one-shot binary
(`bins/zshrs-recorder.rs`, `--features recorder`) that runs the user's
init under runtime AOP and ships the captured bundle to the daemon via
`recorder_ingest`. The user re-runs the recorder when `.zshrc` /
plugins change. See `docs/RECORDER.md` for the recorder design and
`daemon/ops.rs:627` for the deletion comment.

The daemon's role narrowed from "active analyzer" to "passive
canonical store + queryable catalog":

- **Recorder ingest** (`recorder_ingest` op) folds a `RecorderBundle`
  into the canonical engine, replaces every subsystem's row set
  for the bundle's `shell_id`, hydrates the SQLite mirror, and
  broadcasts `recorder_ingested` to opted-in subscribers.
- **fsnotify** still watches files the recorder registered through
  `source_resolve`. When a watched file changes, the daemon emits
  `shard_updated` so subscribed clients know to re-mmap, but does
  NOT re-derive state — the user must re-run the recorder.
- **Source resolver** (`source_resolve` op) is the only on-demand
  parse path the daemon still owns: client requests "give me the
  bytecode for FILE@(mtime, inode)", daemon hits or compiles + caches.
  No transitive auto-walk; whatever the user explicitly sources gets
  registered, nothing else.

Captured-state coverage (per `daemon/definitions.rs:KNOWN_KINDS`):

| Subsystem | Recorded | Queried via |
|---|---|---|
| `alias` / `galias` / `salias` | recorder AOP on the alias dispatcher | `definitions_query` |
| `function` / `function_autoload` | recorder AOP on function-define + `autoload` | `definitions_query` |
| `env` / `params` / `params_typed` | recorder AOP on assign + typeset, with `ParamAttrs` bitset | `definitions_query` |
| `bindkey` / `compdef` / `zle` | recorder AOP on each builtin | `definitions_query` |
| `zstyle` / `zmodload` / `setopt` | same | `definitions_query` |
| `path` / `fpath` / `manpath` | path edits captured + ordered | `definitions_query` |
| `named_dir` (`hash -d`) | recorder AOP on `hash -d` | `definitions_query` |
| `trap` / `sched` / `source` | recorder AOP | `definitions_query` |
| `completion` files in fpath | recorder synthesizes one `completion` event per `_*` file when an fpath dir is added | `definitions_query` |

`shell_id` federation lets bash/fish/ksh/etc. recorders write to the
same catalog with their own identity; cross-shell `definitions_diff`
compares any two. See `docs/SHELL_IDS.md` and
`docs/DAEMON_AS_SERVICE.md` §"Third-party shell recorders".

**Special parameters served by daemon** (`_comps` / `_services` /
`_patcomps` / `_describe_handlers`) come from recorder-captured
`compdef` rows folded into the canonical engine. `eval $(zcache
export _comps)` rebuilds them in a new shell. The four-table set is
listed at `daemon/builtins.rs:704` + `daemon/export.rs:480`.

**`fork+exec`'d user commands** (`find`, `ls`, `rg`, `grep`, etc.)
are user code, not shell-internal state — they run normally in the
client and are out of scope for the recorder.

### Source / dot interception and file registry

The `source` / `.` builtins are daemon-aware. Each call routes through
`source_resolve`:

```
client: source /Users/wizard/.zpwr/local/.tokens.sh
    │
    ↓  IPC: {"op":"source_resolve","path":"…/.tokens.sh","mtime":N,"inode":M}
    │
daemon: lookup compiled_files WHERE path = …
    HIT  (mtime+inode match):  return cached shard_path + generation
    MISS:                      parse + bytecode-compile + insert + return
    STALE (mtime/inode differ): rebuild + atomic-rename + return new generation
    │
    ↓
client: mmap shard, execute in current process
```

`source_resolve` is **on-demand only** — daemon does not transitively
auto-walk the closure reachable from `.zshrc`. (The original spec did;
that pass was removed with the AST-walk pipeline. Recorder runs cover
the transitive-closure observation by recording every `source`
event during init.) The compiled_files registry still serves cache-hit
fast-path for any file the user explicitly sources, plus fsnotify
re-validation when a watched file changes.

**Sensitive content.** Some sourced files contain secrets (API keys,
passwords, `export AWS_SECRET_ACCESS_KEY=…`). Two enforcement points:

1. `~/.zshrs/` is `0700` (user-only); files inside `0600`. Set
   at daemon startup, verified by `zcache verify`. Drift → `WARN` in
   log + refusal to attach for non-owner clients.
2. `compiled_files.sensitive` flag set by `daemon/source_resolver.rs`
   `is_sensitive` heuristic: file path matches
   `*tokens*` / `*secret*` / `*credentials*` / `*.env*`, or content
   matches `AWS_SECRET` / `API_KEY=` / `PASSWORD=` /
   `SECRET_ACCESS_KEY` / `PRIVATE_KEY`. Sensitive shards are
   `O_NOFOLLOW`-opened + `MAP_PRIVATE`-mmap'd + excluded from
   `zcache export --all` unless `--include-sensitive` is passed.

### Compat surface and zpwr-scale validation target

Daemon design is validated against the `~/.zpwr` codebase as the bedrock real-world load — anything that can't handle this is a design failure. Concrete numbers from the user's machine:

| Measurement | Value |
|---|---|
| `.zshrc` line count | 889 |
| `~/.zpwr` total shell LOC (`.zsh` + `.sh`) | ~1.6M |
| `~/.zpwr` shell file count | 579 |
| Existing `.zwc` files in `~/.zpwr` | 40 |
| `~/.zpwr/local/.zcompdump-zpwr-MenkeTechnologies` | 753 KB |
| Same, zcompiled (`.zwc`) | 1.8 MB |
| `~/.zpwr/local/.zpwr-MenkeTechnologies-history` | 25 MB (478k commands) |
| `~/.zpwr/local/.tokens.sh` (sensitive, pre-zwc'd) | 12 KB |
| `~/.zpwr/local/.common_aliases` | 92 KB |

The recorder must absorb all of this on a recorder run (one-shot per
`.zshrc`/plugin change). Subsequent shell boots see fast cold-start
because the canonical state is already in the daemon's rkyv shards.

**Plugin manager interop.** zpwr's `.zshrc` switches between plugin
managers via `$ZPWR_PLUGIN_MANAGER` (zinit, antigen, zplug, antibody,
oh-my-zsh, sheldon, znap, zgenom, zcomet, zr, direct git-clone). The
daemon does not parse any of these managers' syntax — the recorder
runs the user's actual `.zshrc` under their actual plugin manager and
captures whatever runtime state mutations result. So the compat
surface is whatever the user's chosen manager actually does at
runtime, end-state-equivalent to what the recorder observed.

zinit-turbo-style deferral is irrelevant under recorder ingest —
deferred subshells flush their state through the same dispatchers,
which are AOP-instrumented, so end-state captured by the recorder
matches the post-defer steady state. See `docs/RECORDER.md` §"Why
this beats AST analysis at zinit scale".

**`.zwc` files: invisible to recorder, importable on demand, encouraged
to delete.** The recorder, the source resolver, and fsnotify all
filter out `.zwc` files and treat them as if they don't exist. The
recorder only ever observes source-file sources (`.zsh`, `.sh`). `.zwc`
files in `~/.zpwr` (or anywhere else) sit on disk untouched and unread
by any auto-driven code path. They never feed into cold-start, never
participate in fsnotify, never count toward "what's in fpath."

Once a user is on zshrs, every `.zwc` and `.zcompdump-*` file is **dead disk litter that contributes zero speed** — no longer doing any work, just consuming bytes and confusing future debugging.

`.zwc`'s entire reason to exist is "skip the source-parse step on cold load" — but zshrs clients never parse source on cold load. The daemon parsed it once, serialized into rkyv, and clients mmap the rkyv shard. The cold path is mmap + index + execute. There's no parse step for `.zwc` to short-circuit, so even if zshrs *did* read `.zwc`, it would be slower than the rkyv path (`.zwc` deserialization → AST → re-execute is more work than mmap → indexed bytecode → execute). `.zcompdump` has the same problem at one layer up (compinit-cache vs daemon-served `_comps`).

zshrs encourages cleanup:

```
zcache clean zwc                    # find and delete every .zwc inside known scope (fpath, plugin trees,
                                    # source-statement targets, autoload paths, ~/.zpwr, ~/.zsh, etc.)
zcache clean zwc --dry-run          # report what would be deleted, delete nothing
zcache clean zcompdump              # find and delete every .zcompdump* file (default scope: $HOME, $ZDOTDIR,
                                    # $ZPWR_LOCAL, $XDG_CACHE_HOME)
zcache clean legacy                 # zwc + zcompdump together; the standard "I am fully on zshrs" cleanup
zcache verify                       # already exists; reports .zwc/.zcompdump presence as a WARN with the
                                    # cleanup hint, since their existence implies stale legacy artifacts
```

Out-of-scope dirs are never touched (no recursive `find ~ -name '*.zwc'`). Daemon walks only the directories it already knows about from the user's `.zshrc` analysis. `--dry-run` shows the full list before commit. No confirmation prompt at delete time (per CLAUDE.md "no friction"). `zcache verify` runs as part of `zcache info` and flags litter every time the user looks at cache state, gently surfacing the cleanup verb without nagging.

Once cleaned, the user's daily-driver disk footprint for shell artifacts is just `~/.zshrs/` — single directory, daemon-owned, queryable, exportable. No more 40 `.zwc` files scattered through `~/.zpwr`, no more 1.8 MB `.zcompdump.zwc` orphan, no more `~/.zcompdump*` accumulating across versions.

**Why no auto-import:** `.zwc` (and `.zcompdump`) files can be arbitrarily stale relative to the source they were compiled from. Picking them up automatically would let stale bytecode bleed into the daemon's canonical view, masking real source changes and producing the same "completion is stale, restart shell, still stale" failure mode that motivates the daemon architecture in the first place. The daemon's source-file-is-authoritative rule means it always parses fresh from `.zsh` / `.sh` and never trusts a pre-compiled artifact unless the user explicitly says so.

**On-demand import (user-invoked only):**

```
zcache import zwc <path>            # ingest a single .zwc — daemon validates against the adjacent
                                    # source file (mtime+hash); skips with WARN if stale; on match,
                                    # uses the .zwc to skip the source-parse pass for that file
zcache import zwc --tree <dir>      # walk a directory, import every .zwc that has a fresh adjacent
                                    # source file; stale ones reported and skipped
zcache import zcompdump <path>      # ingest a .zcompdump as compdef seed (validated against current
                                    # fpath; entries pointing at non-existent functions are dropped)
```

Both verbs are explicit user choice, never run automatically. Both validate freshness before merging:

- `.zwc` ingest: daemon stats the adjacent `.zsh` / `.sh` source; if mtime newer than `.zwc`, skip with `WARN: stale .zwc, will reparse from source`. If fresh, ingest the bytecode equivalence and skip the parse step for that file.
- `.zcompdump` ingest: daemon walks every entry; if the referenced function file doesn't exist or has a newer mtime, drop it; only fresh entries are merged.

Conflict resolution: incoming entries that disagree with daemon's current canonical state report a merge plan; `--force` overrides, default is skip-with-WARN.

This preserves the optionality (user can opt in to skip parse work for a known-fresh prebuilt cache) without the auto-staleness trap.

**History migration.** zpwr's 25 MB / 478 k-command history file (`~/.zpwr/local/.zpwr-MenkeTechnologies-history`) ingests once into the daemon's `history.db` via `zcache import history <path>`. The legacy zsh `HISTFILE=` setting becomes a no-op once migrated — daemon owns the canonical store. Backwards-export to legacy zsh format available via `zcache export history --format zsh-histfile` for round-trip.

**`compinit`'s `.zcompdump-…` files.** Daemon ignores existing `.zcompdump` on the first run (the rkyv corpus is canonical), but `zcache import zcompdump <path>` provides a one-shot ingest path. After migration, the legacy `.zcompdump` files remain on disk untouched until the user removes them.

**Acceptance against zpwr.** Cold-cache full-corpus build for `~/.zpwr` (1.6 M LOC, 579 files): target <60 s clean compile. Warm-cache cold-start of a new shell: target <10 ms. After migration, every existing user workflow that worked under zsh+zpwr+zinit+p10k must work under zshrs+daemon, with the speedups described in the comparison table at the top of this doc.

### Promoting client-local changes to daemon canonical

Clients can push their local overlay state up to the daemon to become the new starting-state for **future** shells. This makes the overlay a staging area for what may eventually become canonical, with the user explicitly deciding what gets promoted.

**Direction of effect:**

- Pushing client's `$PATH` modification → daemon updates canonical PATH → command hash table rebuilt daemon-side → next shell boots with new PATH already populated.
- The pushing shell itself already has the new PATH via its overlay; the push is for the benefit of future shells.
- Existing other shells stay on their boot-time snapshot unless they explicitly opt in to canonical-change events via subscription.

**Mechanism:** new IPC op + event + builtin family:

- IPC op: `{"op":"push_canonical","args":{"subsystem":"path","value":["/usr/bin","/usr/local/bin","/opt/foo/bin",…]}}`
- IPC event: `{"event":"canonical_changed","subsystem":"path","generation":N}` — fired to subscribers after daemon commits.
- Builtin: `zsync` family.

**`zsync` builtin (added to the z\* family):**

```
zsync up path                       # push current $PATH to daemon canonical
zsync up fpath                      # push current $FPATH
zsync up named_dir <name…>          # push named-directory entries (hash -d)
zsync up named_dir --all
zsync up alias <name…>              # push specific alias(es)
zsync up alias --all                # push all aliases from overlay
zsync up function <name…>           # push function definition to daemon
zsync up compdef <name…>            # push compdef registration
zsync up env <var…>                 # push exported env var(s)
zsync up params <var…>              # push non-exported shell parameter(s)
zsync up zstyle <pattern…>          # push zstyle declarations
zsync up zstyle --all
zsync up bindkey <key…>             # push keybindings
zsync up bindkey --all
zsync up setopt <option…>           # push shell options
zsync up zmodload <module…>         # push module load declarations
zsync up --all                      # promote everything in overlay to canonical
zsync diff                          # show overlay-vs-canonical for all subsystems
zsync diff <subsystem>              # focused diff
zsync watch <subsystem…>            # subscribe to canonical_changed events for these subsystems
zsync pull <subsystem>              # explicit pull: refresh local state from daemon canonical (opt-in mid-session refresh; breaks snapshot rule on user request)
```

**Visibility semantics for currently-running shells:**

- Snapshot-at-boot is the default. Other running shells see their boot-time canonical, not the new one.
- `zsync watch path` subscribes to `canonical_changed.path` events. On match, the shell can react: log a notice, prompt the user, or call `zsync pull path` to opt in to the new state mid-session.
- Without subscription, running shells stay frozen until `exec zshrs` or explicit `zsync pull`.

**Daemon-side commit flow on `push_canonical`:**

1. Validate the pushed value (sane format, dirs exist for PATH/FPATH, no duplicate keys, etc.).
2. Update canonical state (in-memory + persisted to a daemon-managed config shard).
3. Rebuild any derived hashtable that depends on the changed subsystem (e.g., command hash table for PATH change).
4. Atomic-rename the affected shard, bump generation, update `index.rkyv`.
5. Emit `canonical_changed` event to subscribers.

**What this gets you:** explicit control over what becomes "the way it is" for future shells. `path+=(/opt/foo/bin); zsync up path` is a session-action that takes effect for the next 100 tmux panes the user opens. Without `zsync up`, the path mod dies with this shell.

### Universal cache dump / view / export

For debugging, backup, migration, and integration with legacy tooling, the daemon can serialize **any** of its caches in multiple formats. Two user-facing verbs:

- `zcache view <target>` — pretty-print to stdout (default format = human-readable text; `--format json|yaml|disasm|…` for structured)
- `zcache export <target> [--out <path>]` — write to file (default format = native rkyv binary; `--format` for alternatives)

Both are thin IPC wrappers over daemon ops `view_cache` / `export_cache`. Daemon does the serialization work; client only paints stdout or writes to disk.

**Targets** (what can be dumped):

| Target | Description |
|--------|-------------|
| `path` | Canonical `$PATH` |
| `fpath` | Canonical `$FPATH` |
| `manpath`, `infopath`, `cdpath`, `ld_library_path` | Resolved values |
| `named_dir` | `hash -d` table |
| `command_hash` | Command name → executable path table |
| `autoload_table` | Function name → file path table |
| `aliases` | Alias table |
| `galiases` / `saliases` | Global aliases (`alias -g`) / suffix aliases (`alias -s`) |
| `functions [<name>]` | All function bytecode, or one named function (+ disassembly with `--format disasm`) |
| `compdef` / `_comps` | Completion handler dispatch table |
| `_services`, `_patcomps`, `_describe_handlers` | Sub-handler tables |
| `zstyle` | zstyle context-pattern → key-value registry |
| `bindkey` | Keybinding map |
| `setopt` | Option mask |
| `zmodload` | Loaded module set |
| `env` | Exported env vars (visible to subprocesses) |
| `params` | Non-exported shell parameters — scalar, array, assoc — this-shell-only |
| `theme` | Resolved theme templates (PROMPT, RPROMPT, palette) |
| `history` | Command history (with `--filter` for FTS query, `--range` for time range) |
| `entry_stats` | Frecency / call counts / total time |
| `subscriptions` | Active pub/sub subscriptions (this shell or `--all`) |
| `shells` | Live shell registry (same data as `zls`) |
| `plugins` | Installed plugins, deps, versions, enabled state |
| `shard <name>` | Specific rkyv shard contents |
| `index` | `index.rkyv` lookup table |
| `catalog` | Full `catalog.db` dump |
| `script <path>` | Bytecode for a cached `zshrs FILE` script |
| `sourced <path>` | Bytecode for a single sourced file (with `--all` for registry of every file ever sourced) |
| `compiled_files` | Full compiled_files table — every file the daemon has bytecode-cached, with kind, mtime, hash, sensitive flag, parent_paths |
| `zcompdump` | Synthetic `.zcompdump` for legacy tools (only valid as export, not view) |
| `daemon_state` | Full daemon state for debugging (sizes, queues, lock states, in-flight jobs) |

**Formats** (`--format <fmt>`):

| Format | Use | Valid targets |
|--------|-----|---------------|
| `sh` (default for `export` on shell-state targets) | Eval-compatible zsh script: `eval $(zcache export <target>)` resets overlay to canonical. Includes wipe prefix unless `--additive` | path/fpath/manpath/named_dir/aliases/galiases/saliases/functions/_comps/_services/_patcomps/_describe_handlers/zstyle/bindkey/setopt/zmodload/env/params/theme/command_hash/autoload_table |
| `text` (default for `view`) | Human-readable pretty-print | All targets |
| `json` | Machine-readable structured | All targets |
| `yaml` | Human + machine readable | All targets |
| `native` | rkyv zero-copy binary | All targets (default for `export` on binary-only targets: shard/index/catalog) |
| `sql` | SQL INSERT statements | catalog/entries/entry_stats/plugins/history |
| `csv` | Tabular | history/entry_stats/shells/plugins |
| `zcompdump` | Legacy zsh compinit format (byte-compatible) | compdef/_comps/_services/_patcomps/_describe_handlers (combined) |
| `disasm` | Disassembled bytecode (mnemonic + operands) | function/script/shard |

**Examples:**

```
zcache view path                          # pretty-print resolved $PATH, one dir per line with exists/missing status
zcache view command_hash --filter 'git*'  # commands matching glob, with executable paths
zcache view function _git --format disasm # disassembled bytecode for _git
zcache view history --filter 'cargo' --range 7d  # last 7 days of cargo commands
zcache view subscriptions --all           # every active subscription across every shell

zcache export path --format sh            # path=(...) suitable for sourcing
zcache export catalog --out ~/backup.db   # full catalog backup
zcache export shard zpwr --format json    # zpwr shard as JSON
zcache export zcompdump                   # legacy .zcompdump for plugin compat
zcache export daemon_state --format yaml  # full daemon state for bug reports
zcache export --all --out ~/zshrs-backup.tar.zst  # snapshot every cache target into one archive
```

**Import** (one-shot, limited):

```
zcache import zcompdump ~/.zcompdump      # ingest legacy compinit cache (migration assist)
zcache import catalog ~/backup.db         # restore catalog (loses entry_stats unless backup includes them)
zcache import shard <name> /path/to.rkyv  # restore specific shard
zcache import --all ~/zshrs-backup.tar.zst  # restore from full snapshot
```

Imports validate format + version before merging. Conflicts (incoming entry differs from current canonical) report a merge plan and require `--force` to override.

This makes `~/.zshrs/` fully introspectable and portable. Every byte of canonical state can be exported in a format suited to the consumer — text for humans, JSON for scripts, sh for replayable backups, zcompdump for legacy compat, native rkyv for binary-fast portability. Diagnosing a misbehaving completion, comparing two users' caches, sharing a daemon-built fpath with a colleague, and migrating from zsh+zinit are all `zcache export` + `zcache import` operations.

### IPC wire format

Length-prefixed JSON over `~/.zshrs/daemon.sock`. Each frame:

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

Authoritative source: `daemon/ops.rs:OP_NAMES` — add a row here when
adding an op there. See `docs/DAEMON_AS_SERVICE.md` for the full
HTTP-surfaced contract per op.

| Area | Op | Purpose |
|---|---|---|
| meta     | `info` | daemon stats, catalog stats, uptime, paths |
| meta     | `ping` | round-trip latency + pong |
| meta     | `daemon` | status/stop/restart control verb |
| meta     | `register` | session register (tag/cwd/argv0); implicit on connect |
| meta     | `doctor` | full diagnostic report |
| meta     | `verify` | integrity scan on shards + catalog |
| meta     | `compact` | vacuum catalog.db, dedup shards |
| meta     | `clean` | unlink shards / index / cache by `target` |
| meta     | `metrics` | Prometheus-shaped metrics as JSON |
| meta     | `log_level` / `log_rotate` / `log_stats` | tracing controls |
| meta     | `config_get` / `config_set` / `config_list` | runtime config knobs |
| meta     | `stats_flush` | merge batched per-call stats into `entry_stats` |
| meta     | `replay_log` | per-shell replay log read-back |
| recorder | `recorder_ingest` | ingest a `RecorderBundle`; replaces all rows for the bundle's `shell_id`; broadcasts `recorder_ingested` |
| caches   | `autoload_prewarm` | compile every `_*` completer on the given (or recorded) `$fpath` into `autoloads.rkyv`. The daemon spawns `zshrs --prewarm-autoloads` rather than compiling in-process: the parser and bytecode compiler live in the `zshrs` crate, which depends on this one, and `parse()` walks process-global lexer state that must not be shared. CLI: `zd prewarm [DIR ...]` |
| canon    | `canonical_hydrate_view` | rebuild SQLite mirror of canonical |
| canon    | `push_canonical` / `pull_canonical` / `diff_canonical` | per-shell overlay promote / fetch / diff |
| defs     | `definitions_query` | federated catalog query (filter by kind/name/prefix/shell_id) |
| defs     | `definitions_kinds` | list every populated kind |
| defs     | `definitions_emit` | single-record write (third-party shell recorders) |
| defs     | `definitions_diff` | cross-shell diff between two shell_ids |
| defs     | `definitions_subscribe` / `definitions_unsubscribe` | opt in/out of `recorder_ingested` events |
| watch    | `watch_subscribe` / `watch_unsubscribe` / `watch_list` | refcounted fsnotify subscriptions |
| watch    | `watcher_stats` | fsnotify watcher counts + state |
| watch    | `fpath_changed` | client tells daemon a new fpath dir was added |
| event    | `subscribe` / `unsubscribe` | pubsub pattern subscription |
| event    | `subscription_set_paused` | pause/resume a subscription |
| event    | `publish` | publish a pubsub event |
| event    | `subscribe_shard` | per-shard update push |
| shell    | `list_shells` | every registered session (powers `zls`) |
| shell    | `tag` / `untag` | self-tag for routing |
| shell    | `send` | `zsend` dispatch (single / broadcast / by tag / by user) |
| shell    | `cmd_started` / `cmd_result` | long-cmd tracking + result event |
| shell    | `notify` | `znotify` OSC-9 / status-line message |
| ask      | `ask_ask` / `ask_pending` / `ask_take` / `ask_dismiss` / `ask_response` | pull-mode UI queue (see `zask` section) |
| cache    | `cache_put` / `cache_get` / `cache_del` / `cache_list` / `cache_stats` | persistent KV store |
| artifact | `artifact_put` / `artifact_get` / `artifact_get_by_digest` / `artifact_list` / `artifact_gc` | sha256-content-addressed cache |
| job      | `job_submit` / `job_status` / `job_output` / `job_list` / `job_kill` / `job_cancel` / `job_wait` / `job_input` / `job_resize` | tokio task queue (`job_input` writes to PTY master fd; `job_resize` propagates TIOCSWINSZ) |
| schedule | `schedule_add` / `schedule_add_once` / `schedule_remove` / `schedule_list` | cron-equivalent |
| lock     | `lock_acquire` / `lock_try_acquire` / `lock_release` / `lock_list` | named cross-process mutex |
| snapshot | `snapshot_save` / `snapshot_list` / `snapshot_load` / `snapshot_diff` | tag-based canonical state captures |
| source   | `source_resolve` | bytecode-cache lookup for `source FILE` / `. FILE` |
| source   | `load_script` | cold-load `zshrs FILE` |
| source   | `import_zwc` / `import_zcompdump` / `import_history` / `import_catalog` / `import_shard` / `import_all` | user-explicit legacy / backup ingest |
| source   | `export` / `view` / `export_all` / `export_catalog` / `export_shard` / `export_zcompdump` | catalog dump / human view |
| history  | `history_query` / `history_append` | FTS search + write |
| compsys  | `complete` / `suggest` / `highlight` | per-keystroke ops (tab completion enumeration / autosuggest / syntax highlight) |
| compsys  | `keys` | iterate keys for a daemon-served special parameter |

### Async event types (daemon → client)

| Event | Trigger |
|-------|---------|
| `shard_updated` | Daemon swapped shard with newer generation |
| `rebuild_complete` | Async compile job finished |
| `canonical_changed` | Daemon canonical state for a subsystem (path / fpath / alias / named_dir / zstyle / bindkey / setopt / zmodload) was promoted by some client; subscribers can `zsync pull` if they want to track it mid-session |
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

# Universal cache dump/export/view — every named target is its own subcommand,
# accepting common flags: [--format <fmt>] [--filter <pat>] [--out <path>] [--all]
zcache view   <target> [flags]      # pretty-print to stdout (default --format text)
zcache export <target> [flags]      # serialize as eval-compatible zsh script to stdout (default --format sh)
zcache import <target> <path>       # ingest external file (stale-validated; --force to override)
zcache import zwc <path>            # on-demand .zwc ingest; daemon validates adjacent source freshness
zcache import zwc --tree <dir>      # walk dir, import every .zwc with fresh adjacent source
zcache import zcompdump <path>      # on-demand .zcompdump ingest; entries validated against current fpath
zcache list                         # list every supported export target

# CRITICAL: `zcache export` default output is eval-compatible. The canonical reset pattern is:
#     eval $(zcache export <target>)
# This restores the target's full canonical state into the current shell with no parser, no
# special importer, no intermediate format. Use case: get back to canonical starting state
# after session experimentation, without exec'ing a new shell (preserves $$, fds, cwd, history,
# job table). Whatever zsh syntax recreates the state, that's what `zcache export` emits.
#
# Default semantics include a wipe prefix so eval truly RESETS overlay back to canonical:
#     zcache export aliases        # emits: unalias -m '*'  followed by  alias foo='bar'  ...
#     zcache export _comps         # emits: unset _comps    followed by  typeset -gA _comps; _comps[git]=_git ...
#     zcache export bindkey        # emits: bindkey -d      followed by  bindkey '^A' beginning-of-line ...
# To suppress the wipe and emit additive-only:
#     zcache export aliases --additive
#
# Examples:
#     eval $(zcache export aliases)        # reset alias table to canonical
#     eval $(zcache export path)           # reset $PATH to canonical
#     eval $(zcache export named_dir)      # reset hash -d entries to canonical
#     eval $(zcache export functions)      # redefine every canonical function
#     eval $(zcache export _comps)         # reset completion handler dispatch to canonical
#     eval $(zcache export zstyle)         # reset all zstyle declarations
#     eval $(zcache export bindkey)        # reset keybindings
#     eval $(zcache export setopt)         # reset option mask
#     eval $(zcache export env)            # reset exported env vars (emits: export FOO=bar ...)
#     eval $(zcache export params)         # reset non-exported shell parameters (emits: typeset -g FOO=bar / typeset -ga ARR=(...) / typeset -gA MAP=(k v) per type)
#     eval $(zcache export --all-state)    # full shell-state reset (everything eval-compat in one go)
#                                          # — equivalent to `exec zshrs` minus the exec
#
# Targets that are NOT eval-compat (binary-only or inspection-only): shard, index, catalog,
# zcompdump, daemon_state, history, entry_stats, subscriptions, shells, plugins. For these,
# `zcache export` requires explicit --format native|json|yaml|sql|csv and refuses default-sh.

# Named export targets (each is a discoverable subcommand of `zcache export` and `zcache view`)
zcache export path                  # canonical $PATH
zcache export fpath                 # canonical $FPATH
zcache export manpath
zcache export infopath
zcache export cdpath
zcache export ld_library_path
zcache export named_dir             # hash -d table
zcache export command_hash          # command_name → executable_path
zcache export autoload_table        # function_name → file_path
zcache export aliases               # alias table
zcache export galiases              # global aliases (alias -g)
zcache export saliases              # suffix aliases (alias -s)
zcache export functions [<name>]    # all function bytecode, or one named function
zcache export _comps                # completion handler dispatch table
zcache export _services
zcache export _patcomps
zcache export _describe_handlers
zcache export zstyle                # zstyle context-pattern → key-value registry
zcache export bindkey               # keybinding map
zcache export setopt                # shell option mask
zcache export zmodload              # loaded module set
zcache export env                   # exported env vars (visible to subprocesses)
zcache export params                # non-exported shell parameters (scalar/array/assoc, this-shell-only)
zcache export theme                 # resolved theme templates
zcache export history [--filter <q>] [--range <r>]
zcache export entry_stats           # frecency / call counts / total time
zcache export subscriptions [--all]
zcache export shells                # live shell registry (same as zls)
zcache export plugins               # installed plugins, deps, versions
zcache export shard <name>          # specific rkyv shard
zcache export index                 # index.rkyv lookup table
zcache export catalog               # full catalog.db
zcache export script <path>         # bytecode for a cached `zshrs FILE`
zcache export sourced <path>        # bytecode for a sourced file (single)
zcache export sourced --all         # registry of every sourced file with mtime/hash/sensitive flag
zcache export compiled_files        # full compiled_files table dump
zcache export zcompdump             # synthetic .zcompdump for legacy tools
zcache export daemon_state          # full daemon state for debugging
zcache export --all [--out <path>]  # snapshot every target into one archive

# `zcache view` has identical target surface, default format = text:
zcache view path
zcache view aliases
zcache view _comps --filter 'git*'
zcache view function _git --format disasm
zcache view history --filter 'cargo' --range 7d
# … etc, every export target is also a view target

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

# State promotion (overlay → daemon canonical for future shells)
zsync up <subsystem> [name…]        # promote local overlay to canonical (path/fpath/named_dir/alias/function/compdef/env/zstyle/bindkey/setopt/zmodload)
zsync up --all                      # promote everything
zsync diff [subsystem]              # show overlay-vs-canonical
zsync watch <subsystem…>            # subscribe to canonical_changed events
zsync pull <subsystem>              # opt-in mid-session refresh from canonical (breaks snapshot rule on user request)

# Daemon log inspection
zlog                                # default: live tail of ~/.zshrs/zshrs.log
zlog tail [-n N] [--follow]
zlog grep <pattern> [--rotated]
zlog level [<new_level>] [<module=level>…]
zlog clear
zlog rotate
zlog path
zlog stats
```

Subscription pattern grammar: `<scope>.<topic>`.

**Scopes:** `shell:<id>` | `tag:<name>` | `user:<name>` (root only) | `*`
**Topics:** `commands` | `chpwd` | `prompt` | `precmd` | `preexec` | `exit` | `signal` | `error` | `cd_history` | `aliases_changed` | `tagged` | `untagged`

Examples:

- `zsubscribe shell:42.commands` — pair-programming, audit
- `zsubscribe *.commands` — fleet-wide command logging
- `zsubscribe tag:prod.chpwd` — track cwd of all prod shells
- `zsubscribe shell:1.chpwd` — mirror cwd from shell #1 in this shell

### zshrs ↔ zshrs-daemon: independent processes

The shell and the daemon are **decoupled binaries**. The shell never
runs daemon server code in its own process. The two interact only
over the Unix socket / HTTP listener as a normal client/server.

- `zshrs` — the shell. Talks to the daemon when present, runs in
  vanilla mode when absent.
- `zshrs-daemon` — the daemon. Standalone binary. Started by the user
  via systemd / launchd / brew services / manual.

Vanilla mode = no canonical-state cache, no recorder, no rkyv shards.
Every `.zshrc` invocation re-evaluates from scratch — Rust-fast vanilla
zsh, but rebuilding your house every morning. The daemon is what makes
the cache+canonical-state architecture work; if it's not running, you
get the speed of zshrs's compiled engine but none of the cross-shell
caching wins.

#### Detect-at-startup probe

zshrs probes the daemon socket once at process startup (see
`src/extensions/daemon_presence.rs`). The probe is connect-only — no handshake,
no spawn — so it costs ~tens-of-microseconds when the daemon is up
and a single `stat()` failure when it's down. Result is cached in an
atomic; subsequent IPC sites read it via `daemon_presence::is_present()`
in O(1).

The probe never spawns the daemon. If the user wants the daemon
running, they start it themselves via one of the install paths.

#### Config knob: `~/.zshrs/zshrs.toml`

```toml
[daemon]
# "auto" (default) = probe at startup; use the daemon if alive
# "off"            = skip the probe entirely; pure vanilla zsh mode
# "require"        = probe and WARN if absent (still doesn't spawn)
enabled = "auto"
```

Three modes captured by `daemon_presence::Mode`:

| Mode      | When                                              | Behavior                              |
|-----------|---------------------------------------------------|---------------------------------------|
| Present   | Probe succeeded                                   | All daemon-backed features available  |
| Absent    | Probe failed (auto/require)                       | Degraded — vanilla zsh mode           |
| Disabled  | `[daemon] enabled = "off"`                        | No probe attempted; vanilla forever   |

`require` exists for users who *want* a hard "did I forget to start
the daemon?" signal at every shell launch — it logs a WARN when
absent so the install-and-forget path is loud about misconfiguration.

### Daemon lifecycle (the daemon-binary side)

The daemon is a standalone binary (`zshrs-daemon`) — never spawned by
the zshrs shell. Users start it via one of the install paths shipped
under `examples/`:

- `examples/systemd/zshrs-daemon.service` (Linux, per-user systemd)
- `examples/launchd/com.menketechnologies.zshrs-daemon.plist` (macOS;
  installer at `examples/install-launchd.sh`)
- `examples/brew/zshrs.rb` (`brew services start zshrs` on macOS / Linux)
- Manual: `zshrs-daemon` from the command line

Once running:

- **Singleton enforcement:** daemon takes `flock(LOCK_EX)` on
  `~/.zshrs/daemon.pid` at startup. Second instance sees lock
  held, exits cleanly.
- **Lifetime:** persists across shell sessions; survives logout
  (loginctl-linger on systemd). Stopped via `zcache daemon stop`,
  `systemctl --user stop zshrs-daemon`, `launchctl unload ...plist`,
  `brew services stop zshrs`, or SIGTERM.
- **Crash recovery:** rkyv shards + `catalog.db` are durable on disk.
  After the daemon restarts, clients reconnect and resume from cache.
- **Degraded mode:** if the daemon is not running, zshrs clients fall
  back to source-interp for everything. Cache features become no-ops;
  the shell stays functional. User never blocked. No auto-respawn.

### First-run notification (the one-time exception to no-banner)

The global "no startup banner / no init progress to terminal" rule
has exactly **one** exception: the first-ever `zshrs-daemon` startup
on a machine, when the cold-cache build is starting from zero. This
is a multi-second-to-multi-minute operation depending on corpus size;
running it silently would be confusing and potentially indistinguishable
from a hung process.

**Detection:** "first-ever run" = no `~/.zshrs/index.rkyv` AND
no `~/.zshrs/images/` shards on disk. After the first run
completes, this branch is never taken again on this machine for this
user.

**What gets printed (stderr, single block, on the daemon's controlling
terminal if any — typically empty under systemd / launchd):**

```
zshrs first-run init — daemon spawning, cold cache building.
  scope: ~/.zshrc + transitive sources + $PATH + $FPATH + plugins
  estimated: 579 files, ~1.6M LOC, ~60s on this machine
  background: shells work via source-interp until cache is warm
  log:        ~/.zshrs/zshrs.log
  inspect:    zcache info | zcache jobs | zcache view <target>
  reset:      zcache clean | rm -rf ~/.zshrs/
```

Six lines, factual, no welcome / no congratulations / no emoji / no version stripe. After this block prints, the prompt appears immediately. Daemon continues building in the background; clients run via source-interp fallback until shards atomic-rename in.

**Completion notice:** when the daemon finishes the cold build, it can emit a single-line `znotify` to the originating shell: `daemon ready — future shells <10ms cold-start (took 47s, 17042 entries)`. The user-visible status-line update is a `znotify` not a stdout/stderr write, so it doesn't disrupt whatever the user is currently doing.

**Subsequent runs**: silent. Per the CLAUDE.md global rule, no banner, no progress, no chatter. The first-run notification is one-shot for this user/machine pair, gated by the on-disk first-run-detection check.

**Override flags** (for users who want first-run silent or who want every-run verbose):

- `--quiet-first-run` (or env `ZSHRS_QUIET_FIRST_RUN=1`): suppress the first-run notification block. Everything still goes to log; user just sees an immediate prompt.
- `--verbose-init` (debug-only): show daemon work to stderr on every run, not just first. For testing daemon behavior; not recommended for daily use.

### Long-running command completion notices

Daemon tracks command duration via the `history_append` IPC (clients send `duration_ns` per command). When a command exceeds the long-cmd threshold and completes, daemon pushes a `long_cmd_complete` event to the user's other registered shells. Original shell already knows it finished (it just got control back); the value is alerting *other* tmux panes / ssh sessions where the user is doing parallel work.

**Threshold:** default 30 seconds. Configurable via `ZSHRS_LONG_CMD_THRESHOLD=<seconds>` env var or `zcache config set long_cmd_threshold <seconds>` runtime override. Per-shell overrides via `ZSHRS_LONG_CMD_THRESHOLD` set before shell launch.

**Event payload (`long_cmd_complete`):**

```json
{"event":"long_cmd_complete",
 "from_shell":42,
 "command":"cargo build --release",
 "exit_code":0,
 "duration_ns":492137000000,
 "cwd":"/Users/wizard/RustroverProjects/zshrs",
 "ts_ns":1735305600000000000}
```

**Routing:** by default delivered to all of the user's currently-registered shells *except* the originating shell. Subscribers can refine via patterns: `zsubscribe *.long_cmd_complete` (any shell), `zsubscribe shell:42.long_cmd_complete` (shell 42 only), `zsubscribe tag:dev.long_cmd_complete` (only dev-tagged shells), `zsubscribe --filter 'duration_ns > 600e9' *.long_cmd_complete` (only commands over 10 min).

**Client rendering:** receiving shell renders the event via the existing `znotify` channel (OSC-9 + status-line update). User sees `[shell:42 ✓ cargo build (8m12s)]` in their other shell's status bar without losing focus on what they were doing.

**Companion events** for richer awareness (same routing rules):

- `long_cmd_started` — fires when a command crosses 5s of runtime (not waiting for completion); useful for "I started something heavy, expect it to take a while" pre-warning.
- `long_cmd_failed` — fires on non-zero exit when duration exceeds threshold; same payload + `stderr_tail` field with the last N lines of stderr captured by daemon's command-output ring buffer.
- `long_cmd_signaled` — fires when a long command exits via signal (SIGINT, SIGTERM, SIGKILL).

**Disable:** `ZSHRS_LONG_CMD_NOTICES=0` for users who don't want any of this. Default on.

### `zask` — daemon-queued UI primitives (pull-mode, never interferes with prompt)

> **Naming note:** `zask` (not `zui`) — `zui` is taken by [`zdharma-continuum/zui`](https://github.com/zdharma-continuum/zui), an existing zsh TUI library plugin. Mnemonic: `zask` = the daemon (or another shell) **asks** the user something.

**Hard rule: daemon-pushed UI never interferes with the user's active prompt.** No auto-overlay, no key capture, no cursor moves, no inline interruption. Cross-shell scripts that need user input from another shell don't get to take over that shell's prompt unannounced.

Instead: daemon-pushed UI is **queued**. The user is *notified* (status-line / OSC-9 / unobtrusive bell) that a UI request is pending; the user explicitly activates it on their own time via a keybinding or `zask take`. Until activated, the prompt is untouched and the user keeps typing.

**Flow:**

1. Some script / shell / daemon-job pushes a UI request: `zask --target shell:42 picker --items "..."`.
2. Daemon enqueues the request in shell:42's pending-UI inbox + pushes a `ask:pending` event.
3. shell:42's status-line shows `[zask:1 pending]` (count of queued requests). OSC-9 fires for terminal-native notification. No prompt change, no cursor move, no input capture.
4. User finishes whatever they're typing. When ready, they hit the configured activation key (default `Ctrl-X q`, vi-mode-compatible) or type `zask take`.
5. shell:42 then renders the next pending UI element — but only after the user explicitly engaged. The picker / input / dialog draws above the prompt, captures keystrokes, returns the response. User-initiated, not daemon-imposed.
6. Response routes back over IPC to the originating shell / script.

**Push events (daemon → target client) — informational only, never auto-render:**

| Event | Payload | Client behavior |
|-------|---------|-----------------|
| `ask:pending` | `request_id, kind, from_shell, summary, urgency` | Increment status-line counter, optional OSC-9, write to log. NEVER render UI. |
| `ask:dismissed` | `request_id, reason` | Decrement counter, clear status-line if count==0 |
| `ask:progress` | `request_id, label, percent, eta_ms` | Update status-line bar (which is already a passive surface, doesn't touch prompt). Allowed to update in place because status-line is reserved space, not the prompt line |

UI rendering events (`ask:picker`, `ask:input`, `ask:dialog`, `ask:menu`) are **only sent to the client when the user explicitly takes the request** via `zask take`. The daemon stages the rendering payload in the inbox; the actual render-event ships only when client signals readiness.

**Builtin: `zask`** (top-level, thin IPC wrapper):

```
# Push side — script asks daemon to queue a UI request on a target shell:
zask --target <shell|tag|*> picker --items <list>
zask --target shell:42 picker --items "$(ls)" --multi
zask --target tag:operator input --prompt "username: "
zask --target shell:7 input --prompt "password: " --secret
zask --target shell:42 dialog --message "Deploy to prod?" --options yes,no --urgency critical
zask --target shell:42 menu --title "Select host" --items "host-1 host-2 host-3"

# Progress is the one passive exception (status-line only, no key capture):
zask progress --target shell:42 --label "Building" --percent 47 --eta 30000
zask progress --target shell:42 --request-id <id> --percent 78    # update in place
zask progress --target shell:42 --request-id <id> --done

# Pull side — user manages their own inbox:
zask pending                                  # list queued requests in this shell
zask take                                     # render the oldest pending; blocks for response
zask take <id>                                # render a specific pending request by id
zask dismiss [<id>|--all]                     # decline/cancel pending request(s); originator gets cancelled=true
zask inbox-clear                              # dismiss every pending request in this shell at once
```

**Activation keybinding:** `Ctrl-X q` by default — chained Ctrl-prefix (Ctrl-X then q for "queue"), works identically in vi-insert, vi-cmd, and emacs-insert modes. Chosen to avoid colliding with zsh's existing `^Xu` (undo) and other established `^X*` bindings. **No default keybindings ever use Meta / Alt** — meta keys are unreliable across terminals (macOS Option-key special chars, tmux pass-through quirks, ssh meta-bit stripping, locale-dependent escape sequences). Ctrl combinations and chained-Ctrl chords pass through every terminal cleanly. Bound via `bindkey '^Xq' zask-take` shipped in zshrs's default keymap, plus `bindkey -M vicmd '^Xq' zask-take` so it works post-Esc too. User overrides via `bindkey '<key>' zask-take` or env var `ZSHRS_ZASK_TAKE_KEY=^G` to remap. The widget activates one pending request at a time. Status-line counter decrements on take or dismiss.

**Status-line presentation:** the only daemon→client UI surface that updates without user activation is the status-line / RPROMPT region — explicitly designed-as-passive area. Format: `[zask:N urgent:M]` where N is total pending and M is critical-urgency count. Updates in place, no prompt-line touch. Client treats this as a watched parameter that triggers a status-line redraw, not a prompt redraw.

**Out-of-band rendering (planned):** integration hooks for tmux status-line, kitty's status bar, alacritty title-bar updates, and OSC-9 → macOS Notification Center / Linux libnotify. Daemon emits the same `ask:pending` event; client routes to whichever passive surface the user has configured. None of these touch the active prompt.

**Use cases (revised under the no-prompt-interference rule):**

- **Cross-shell wizard:** script in shell A pushes `zask --target shell:B picker ...`. Shell B's status-line shows `[zask:1 pending]`. User in shell B finishes their current line, hits `Ctrl-X q`, picks. Answer routes back to A. Script in A blocks until user gets around to it (or times out).
- **Pair programming intervention:** remote operator pushes a `ask:dialog`. Local user's status-line shows `[zask:1 urgent:1]`. User finishes typing, hits `Ctrl-X q`, sees the dialog, picks yes/no.
- **Background-job confirmation:** `zjob` supervisor pushes a `ask:dialog`. Status-line surfaces it. User answers when convenient (or `zjob` times out and aborts).
- **`zsync up` conflict resolution:** queued for the originating shell with summary `function foo conflict — file changed since push`. User takes when ready.
- **Long-running ops with progress:** the only UI event allowed to live-update the status-line. `zask progress` updates the status bar without ever touching the prompt.

**Timeouts:** every queued UI request has a default 60-minute timeout. Originator gets `ask:timeout` event if user never engages. Configurable per-request via `--timeout <seconds>` or `--no-timeout`.

**Visibility model:** `zask pending` shows this shell's queue. `zls --ask-pending` shows pending requests across all of the user's registered shells. Every push, take, dismiss, timeout logged to `~/.zshrs/zshrs.log`.

### Daemon logging (every action goes to logfile)

Daemon is fully observable via `~/.zshrs/zshrs.log`. Every action it takes — every cache build, every fsnotify event, every IPC op handled, every shard rename, every error, every plugin discovery, every cross-shell dispatch — is logged. The log is the canonical record of "what did the daemon do" for debugging, post-mortem analysis, and behavior verification.

**What gets logged at INFO level (default):**

- Daemon start / stop / restart with PID + version + config
- First-init begin / end with corpus size + duration
- Cache bust commands received + scope + duration
- Per-shard rebuild jobs: trigger, source paths walked, entries written, atomic-rename complete, generation bumped
- fsnotify events received: path, kind (create/modify/delete/rename), affected shard, action taken
- Client connect / disconnect: client_id, pid, tty, cwd, session_id
- All IPC ops handled: op name, args summary, response code, duration
- `zsync up` canonical promotions: subsystem, value summary, originating client
- `zsend` / `znotify` dispatches: from, to, command/message, delivery status
- Subscription matches: pattern, scope, topic, recipient_count
- Integrity check results from `zcache verify`
- Log rotation events (when rotating to `zshrs.log.1`, file sizes)

**Levels:**

- `ERROR` — daemon-level failures (rebuild crash, lock contention timeout, irrecoverable corruption)
- `WARN` — recoverable issues (stale `.zwc` skipped on import, fsnotify queue overflow, slow IPC client, malformed message dropped)
- `INFO` — default, all the bullet points above
- `DEBUG` — internal state transitions, individual file parses, hash-table slot writes (high volume)
- `TRACE` — every fn entry/exit (extreme volume; for bug repro only)

**Configuration:**

- Default level: `INFO`
- Override via `ZSHRS_LOG=debug` env var or `--log-level <level>` flag at daemon spawn
- Per-module override: `ZSHRS_LOG=info,fsnotify=debug,ipc=trace` (tracing-subscriber compatible)

**Format:** structured `tracing` output. Each line: `[ISO-8601 timestamp] [LEVEL] [module] message {key=value, key=value, …}`. Compatible with `tail -f`, `grep`, and any log-aggregation pipeline. Optional `--log-format json` for structured ingestion.

**Rotation:** ticker rotates `zshrs.log` to `zshrs.log.1` when size hits 10 MB (configurable via `ZSHRS_LOG_MAX_BYTES`). Up to 4 rotated copies kept (`.1` through `.4`); `.4` is purged when `.3` rotates in. Total disk footprint capped at ~50 MB.

**Top-level builtin: `zlog`.** Dedicated builtin for log inspection — top-level for discoverability since this is a daily-use operation:

```
zlog                                # default: zlog tail --follow (live tail of current log)
zlog tail [-n N]                    # tail of current log (default: last 100 lines)
zlog tail --follow                  # live tail, streams new entries as daemon writes
zlog grep <pattern> [--rotated]     # ripgrep current log; --rotated includes .1-.4 archives
zlog level [<new_level>]            # show or set runtime log level (no daemon restart)
zlog level fsnotify=debug,ipc=trace # per-module overrides at runtime
zlog clear                          # truncate current log (rotated copies preserved)
zlog rotate                         # force a rotation now (don't wait for size threshold)
zlog path                           # print absolute path to current log file
zlog stats                          # daemon log self-stats: line counts, size, errors-since-start
```

`zlog` is a thin IPC wrapper; daemon does the file IO and tailing. Client never opens the log file directly. The `zcache log <verb>` form remains as an alias for `zlog <verb>` so `zcache *` discoverability stays intact, but `zlog` is the canonical surface.

### Hard invariants (rejected proposal classes)

- ANY client-side polling loop, timer, fsnotify watcher, SQLite handle for cache — REJECT.
- ANY client-side data-structure walk over rkyv contents — REJECT.
- ANY client-side write to a daemon-owned file — REJECT.
- ANY second daemon instance — REJECT (singleton via `flock` on `daemon.pid`).
- ANY plugin / compsys bytecode baked into the zshrs binary `.text` — REJECT (working set must scale with what's called).
- ANY mandatory daemon (no source-truth fallback) — REJECT.
- ANY daemon spawn under POSIX mode — REJECT.
- ANY z\* builtin without `z` prefix or that shadows upstream zsh — REJECT.
- ANY hydration progress on stderr/stdout — REJECT (`tracing::info!` to log file only).
- ANY scattered per-plugin cache files outside `~/.zshrs/images/` — REJECT.
- ANY removal of `entry_stats` to "simplify" — REJECT.
- ANY auto-consumption of `.zwc` / `.zcompdump` files on daemon scans, fpath walks, fsnotify watches, or plugin-tree enumeration — REJECT. They're invisible to all automatic discovery; only `zcache import zwc|zcompdump <path>` (user-explicit, freshness-validated) may ingest them.
- ANY periodic re-walk of `$PATH` / `$FPATH` / plugin trees / source-statement targets by the daemon — REJECT. Walks happen exactly twice in daemon's life: first init (cold cache) and explicit cache bust (`zcache clean` / `rebuild`). Steady state is fsnotify-driven incremental updates only. No polling, no cron, no "every 5 minutes refresh."
- ANY default keybinding that depends on Meta / Alt — REJECT. Meta keys behave unpredictably across terminals (macOS Option-key chars, tmux pass-through quirks, ssh meta-bit stripping, locale escape sequences). Ctrl-prefixed bindings only for shipped defaults; users can opt into Meta bindings explicitly via `bindkey` if their setup tolerates it.

### Acceptance criteria

Targets — measured by `daemon/bins/zshrs-daemon-bench.rs` for IPC RTT,
unmeasured otherwise (see TODO list at end of this section).

- Cold client launch (daemon already running): <5ms (mmap + connect + handshake).
- Tab completion lookup: ~150-200ns end-to-end (perfect-hash mmap dereference).
- Inline autosuggest: <2ms IPC roundtrip including FTS query.
- Syntax highlight per keystroke: <2ms IPC roundtrip including parse.
- 100 parallel clients share <30 MB RSS attributable to images (page-cache shared across mmaps).
- Per-client cache overhead: <5 MB.
- `~/.zshrc` cold-source: <50ms with cache hit (mmap + replay env log), regardless of file size.
- `zshrs FILE` cold-launch with cache hit: <10ms.
- Recorder ingest: <1s for a 17k-completion zpwr-scale bundle (recorder
  runtime cost is separate; this is daemon-side ingest only).
- Recorder corpus coverage: 100% of `test_data/zsh_functions/` (1200
  functions) captured per `tests/recorder_zsh_functions.rs`.

**TODO — none of these are CI-asserted yet.** Per-criterion enforcement
hasn't shipped. `zshrs-daemon-bench` measures the IPC RTT criteria but
doesn't compare against thresholds; an `--assert-acceptance` mode that
fails when p50 > target would close the loop. RSS + cold-source
criteria need separate harnesses.

Removed acceptance criteria (no longer applicable):
- `zcache rebuild` <30s — `rebuild` op was deleted with the AST-walk
  pipeline; recorder runs replace this.
- Per-shard rebuild ~100-500ms — same.
- Daemon spawn-on-demand <50ms — daemon is no longer spawned by the
  zshrs shell. Started independently by user / systemd / launchd /
  brew services. There is no "client cold-start without daemon"
  measurement to make; if the daemon isn't running, clients run in
  degraded mode (source-interp, no cache).
- POSIX mode never spawns daemon — moot for the same reason.

