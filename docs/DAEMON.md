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

For legacy tooling that introspects `.zcompdump` directly (some plugin patterns, backup scripts, p10k cache-staleness probes, parallel zsh sessions sharing the cache), the daemon can synthesize a valid `.zcompdump` file on demand from its canonical state. Triggered by `zcache export zcompdump [path]` or the `export_zcompdump` IPC op. Optionally emits the zcompiled `.zcompdump.zwc` form too. The synthesized file is byte-compatible with what `compinit` would have produced, so legacy consumers don't notice the difference. Not generated automatically — opt-in only, on user request.

### Starting state served by daemon (PATH, FPATH, hash tables, etc.)

Daemon parses the user's `.zshrc` AND every plugin it sources (zinit-loaded plugins, oh-my-zsh-loaded plugins, manually-sourced files), evaluates them in an analysis pass, and consolidates the resulting state effects into starting-point caches that all clients consume. The no-walking rule is total: anything the daemon can pre-walk and serve, it does. Clients never `find`, never `glob` over fpath, never enumerate PATH directories, never scan plugin trees.

Plugin discovery happens at the same time as `.zshrc` analysis: daemon walks the user's `.zshrc`, sees zinit/OMZ/source calls, descends into each referenced plugin, parses + bytecode-compiles per-plugin shards (`{hash8}-plugin-{name}.rkyv`), captures every state contribution (alias declarations, function definitions, fpath additions, `compdef` calls, `zstyle` declarations, `bindkey` calls, `setopt` calls, env exports), and folds them into the consolidated starting-point caches below. Each plugin's compiled bytecode lives in its own shard for cache-locality and per-plugin invalidation; the *state effects* of all plugins fold into the unified per-user boot-state image.

Pre-walked state delivered at boot:

| State | Mechanism |
|-------|-----------|
| `$PATH` | Daemon evaluates user dotfiles, resolves `PATH=` / `path+=` / plugin contributions, serves final string |
| Command hash table | Daemon walks every directory in `$PATH`, builds perfect-hash `command_name → absolute_path` table in rkyv. Client `which`/`command -v` = single lookup. `hash -r` = IPC to daemon |
| `$FPATH` | Resolved list of autoload directories, served as a flat array |
| Autoload function table | Daemon walks every `$FPATH` directory, populates `function_name → (shard_id, byte_offset)` in `index.rkyv`. Client `autoload` = hash lookup, never `find`/`glob` |
| `$MANPATH`, `$INFOPATH`, `$CDPATH`, `$LD_LIBRARY_PATH` | Same model — resolved values served at boot |
| **Named-directory hash (`hash -d`)** | Daemon parses all `hash -d name=/path` from `.zshrc` and plugins, serves resolved `name → path` perfect-hash table. `~name` expansion = single lookup. Interactive `hash -d` writes to overlay; `zsync up named_dir` promotes |
| Completion staleness metadata | Daemon scans completion file mtimes; results live in `entries.source_loc` + `entry_stats` |
| Theme initial state | Daemon resolves PROMPT segments, RPROMPT, color palette once; serves the final templates |
| Initial alias table | Daemon parses user `.zshrc` alias declarations, serves resolved table. Interactive `alias foo=bar` writes go to overlay |
| Global aliases (`alias -g`) and suffix aliases (`alias -s`) | Pre-resolved by daemon, served as separate perfect-hash tables |
| Initial shell-options state | `setopt`/`unsetopt` calls in `.zshrc` pre-resolved; client boots with final option mask |
| Initial keybinding table | `bindkey` declarations pre-resolved; client boots with final binding map |
| Loaded modules state | `zmodload` declarations pre-resolved; daemon ensures required modules are available, serves initial loaded-module set |
| Initial environment | `export FOO=bar` in `.zshrc` pre-resolved; client boots with final env |
| `zstyle` registry | All `zstyle` declarations from `.zshrc` and plugins pre-resolved into a daemon-served context-pattern → key-value table |

The mechanism is uniform: daemon parses and evaluates user dotfiles in an analysis pass, captures deterministic state effects, serializes into the user's boot-state shard. Client at boot mmaps the shard, applies state to its process, and is fully initialized. No client-side filesystem walks for shell-internal purposes.

**Determinism boundary:** non-deterministic `.zshrc` fragments — anything that calls `$(date)`, reads `/dev/urandom`, conditionally branches on `$$` or `$RANDOM`, depends on per-shell state — are detected during the analysis pass and emitted as a small per-shell replay log. Client executes those fragments locally at boot. The vast majority of `.zshrc` content is deterministic and gets pre-resolved on the daemon side.

**What's NOT covered by this rule:** `fork+exec`'d user commands (`find`, `ls`, `rg`, `grep`, etc.) are user code, not shell-internal walks. Those run normally in the client. The no-walking rule applies only to shell-internal directory enumeration (PATH/FPATH scans, completion file lookups, autoload resolution, plugin discovery, `hash` table population, theme file reads).

Result: a 172k-line `zpwr` `.zshrc` should cost client cold-start no more than the IPC + mmap + state-apply pass — measured in milliseconds, not seconds. Per-client init cost is independent of `.zshrc` size or fpath cardinality.

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
zsync up env <var…>                 # push env var(s)
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
| `env` | Env var table (canonical) |
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
| `zcompdump` | Synthetic `.zcompdump` for legacy tools (only valid as export, not view) |
| `daemon_state` | Full daemon state for debugging (sizes, queues, lock states, in-flight jobs) |

**Formats** (`--format <fmt>`):

| Format | Use | Valid targets |
|--------|-----|---------------|
| `sh` (default for `export` on shell-state targets) | Eval-compatible zsh script: `eval $(zcache export <target>)` resets overlay to canonical. Includes wipe prefix unless `--additive` | path/fpath/manpath/named_dir/aliases/galiases/saliases/functions/_comps/_services/_patcomps/_describe_handlers/zstyle/bindkey/setopt/zmodload/env/theme/command_hash/autoload_table |
| `text` (default for `view`) | Human-readable pretty-print | All targets |
| `json` | Machine-readable structured | All targets |
| `yaml` | Human + machine readable | All targets |
| `native` | rkyv zero-copy binary | All targets (default for `export` on binary-only targets: shard/index/catalog) |
| `sql` | SQL INSERT statements | catalog/entries/entry_stats/plugins/history |
| `csv` | Tabular | history/entry_stats/shells/plugins |
| `zcompdump` | Legacy zsh compinit format (byte-compatible) | compdef/_comps/_services/_patcomps/_describe_handlers (combined) |
| `zwc` | Zsh-compiled `.zwc` form | function bytecode, compdef table, .zshrc body |
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

This makes `~/.cache/zshrs/` fully introspectable and portable. Every byte of canonical state can be exported in a format suited to the consumer — text for humans, JSON for scripts, sh for replayable backups, zcompdump for legacy compat, native rkyv for binary-fast portability. Diagnosing a misbehaving completion, comparing two users' caches, sharing a daemon-built fpath with a colleague, and migrating from zsh+zinit are all `zcache export` + `zcache import` operations.

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
| `push_canonical` | Promote client overlay state for a subsystem (path/fpath/alias/named_dir/etc.) into daemon canonical for future shells |
| `pull_canonical` | Client opt-in: re-fetch canonical state for a subsystem mid-session |
| `diff_canonical` | Get overlay-vs-canonical diff for inspection |
| `export_zcompdump` | Emit a synthetic `.zcompdump` (and optional `.zcompdump.zwc`) from canonical state for legacy tooling |
| `export_catalog` | Dump `catalog.db` to a portable file |
| `export_shard` | Dump a specific rkyv shard to a portable file |
| `import_zcompdump` | Ingest a legacy `.zcompdump` for migration assist |
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
zcache import <target> <path>       # ingest external file
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
zcache export env                   # canonical env vars
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
zcache export zcompdump [--zwc]     # synthetic .zcompdump for legacy tools
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

