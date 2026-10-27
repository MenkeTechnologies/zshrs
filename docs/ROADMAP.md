# zshrs roadmap — endgame shell

**Status:** Phase F complete (tree-walker dispatch deleted; hand-crafted tests in `tests/no_tree_walker_dispatch.rs` and `tests/tree_walker_absent.rs` pin "no tree walker" as a load-bearing invariant). Phases G–O are the path from "no tree walker" to "v1.0 endgame shell, daily-driver replacement for zsh."

**Audience:** the maintainer (one person, planning in decades). Every phase has discrete acceptance criteria. No phase is complete until its tests pass.

---

## Operating principles

These are the design constraints. Violations are bugs.

1. **No tree walker, ever.** Every `ShellCommand` compiles to `fusevm::Chunk` (`src/extensions/compile_zsh.rs`) and runs on `fusevm::VM`. The deleted dispatch tree (`execute_simple/pipeline/list/compound/command_bg`) does not return. `tests/tree_walker_absent.rs` enforces.

2. **Power-user defaults are non-negotiable.** No startup banner, no init-progress to terminal, no deprecation nags, no "did you mean," no safety prompts on destructive ops, no tip-of-the-day, no `--help`-hints in errors. Errors stay on stderr in `zshrs: <cmd>: <reason>` format. Informational chatter goes to `~/.zshrs/zshrs.log` via `tracing::*`. One-way ratchet — never reintroduce removed banners. **This is not fish.**

3. **Every PR ships behavioral tests.** Pin exact stdout + exit. The Phase F tests are the floor; this plan adds 400+. No invariant, no merge.

4. **Compat with the maintainer's existing world is sacred.** zpwr (172k LOC), zsh-more-completions (47,394 completion files as of 2026-10-27, v0.13.56), the .zshrc, all zinit plugins — these continue working as zshrs evolves. Compat-floor regressions are catastrophic; ship none.

5. **Plan in decades.** This is the endgame shell for a 30+ year horizon. Every architectural decision must survive that. Bytecode formats versioned. SQLite schemas migration-safe. Dependencies vendorable. MSRV pinned. No fashionable churn.

6. **Readable over clever.** Future-maintainer-at-65 must navigate this codebase. Concise, documented, no premature optimization that obscures intent.

7. **zshrs is the god of all processes.** Default new features to "lives inside zshrs as a builtin or host op." External processes only when there's no choice. Forks cost 2-5ms; millions of commands per day; every fork avoided is real wall time.

8. **Synthesis posture, not migration posture.** Anything good fish/nushell/elvish/oil innovate, evaluate for absorption. Endgame means convergence, not just zsh-in-Rust.

---

## Phase G — Compat floor (daily-driver replacement)

Critical path. Until G is done, zshrs cannot replace zsh on the maintainer's machine. Order is dependency-respecting.

### G0 — Friendly-output audit
- Sweep every `println!`/`eprintln!`/`eprint!`/`print!` in the codebase.
- Route each to `tracing::{info,warn,debug}` UNLESS it's in: `--help`/`--version`/`--doctor`/`dbview`/builtin user-invoked output, error reporting on stderr (`zshrs: <cmd>: <reason>` format only), or user-script output.
- Add `#![deny(clippy::print_stdout, clippy::print_stderr)]` lint to `bins/zshrs.rs` so future regressions break the build.
- **Acceptance:** `zshrs -i -f </dev/null` produces zero bytes on stdout/stderr before the prompt.
- **Test:** regression test runs `zshrs -i -f` with stdin closed, asserts output is empty until prompt char.
- **Effort:** 1 day.

### G1 — Real shell arrays
- `compile_simple` detects `ShellWord::ArrayLiteral(elems)` in assignment context, emits N pushes + `Op::MakeArray(N)` + `BUILTIN_SET_ARRAY` (id 287).
- New `BUILTIN_SET_ARRAY` writes `Vec<String>` into `executor.arrays`, not `executor.variables`.
- New `BUILTIN_SET_ASSOC` (id 288) for `declare -A` assoc arrays.
- `compile_word` for `ShellWord::ArrayVar(name, idx)` lowers to `BUILTIN_ARRAY_INDEX` (id 289).
- `ZshrsHost::expand_param` handles `ArrayLength`/`ArrayIndex`/`ArrayAll`/`KEYS` (`${arr[@]}`, `${#arr[@]}`, `${!arr[@]}`).
- **Argv splice (the hard part):** bump fusevm to 0.10.1 — modify `Op::Exec`/`Op::ExecBg`/`Op::CallFunction` to flatten `Value::Array` arguments into argv (one-line `flat_map` semantics change).
- 30+ array tests in `no_tree_walker_dispatch.rs`: indexing, slicing, splice expansion, assoc keys/values, `${arr[@]}` in for-loop, `${arr[@]}` as cmd args, append `arr+=(...)`, `${#arr}` length.
- **Acceptance:** every zsh array idiom in zpwr's source compiles + runs identically.
- **Effort:** 3 days.

### G2 — Autoload + fpath scan via sharded rkyv image cache + zcache builtins
- Per `AOT_DESIGN.md` §0x13: daily-driver bytecode lives in `~/.zshrs/images/{hash8}-{slug}.rkyv` (one image per source root, hash-prefix collision-proof) with top-level `~/.zshrs/index.rkyv` for two-level lookup. Wire `loadautofn` (`src/ported/exec.rs:704`) to: hash fq_name → `index.rkyv` lookup → `(shard_id, generation, byte_offset)` → get-or-mmap the shard image (LRU shard-handle cache; munmap+remmap on generation mismatch) → typed pointer → JIT/interp the chunk in place. ~150-200ns end-to-end. No SQLite hit on the hot path.
- **Source-truth fallback:** image miss / malformed shard / version mismatch / file missing / corruption → main thread silently falls through to source-interp path AND enqueues compile job. User never blocked. Source files are authoritative; image is opportunistic accelerator.
- **Single cache directory** — everything (images, index, catalog, history, log) under `~/.zshrs/`. No XDG split. Full nuke = `rm -rf ~/.zshrs/` (user accepts loss); surgical resets via per-file `rm` or `zcache clean` builtin.
- **Hard invariant: nothing blocks the shell.** All rkyv shard compilation, image writes, index rewrites, catalog hydration, SQLite VACUUM run in the worker pool. Main thread NEVER calls compile pass synchronously. All `zcache` write operations are async by default; `--wait` opts in to blocking.
- **No fsnotify in clients** — user runs 60+ parallel zshrs (Cursor workflow); fsnotify per process = 60+ watchers thundering-herding every edit. fsnotify lives in the daemon only (`daemon/fsnotify.rs`); clients run no watchers. State indexing is done by `zshrs-recorder` (`docs/RECORDER.md`); the `zcache rebuild` verb was removed.
- **Per-shard flock** — `images/{hash8}-{slug}.rkyv.lock` advisory lock (`daemon/shard.rs::lock_shard`), held blocking-exclusive across serialize + `rename`, so concurrent writers of one shard serialize instead of last-rename-wins.
- **Strict shard-rename-then-index-update ordering** — atomic-rename `{name}.rkyv.tmp.{pid}.{tid}` → `{name}.rkyv` FIRST, then rewrite `index.rkyv`. Reverse order would let main read new-index → deref into OLD mmap → corrupt reads. Generation counter on each shard header drives main's munmap+remmap on stale handles.
- **Worker pool partitioned** — general pool (high-priority, `nproc - 1` threads) for user pipelines; cache pool (low-priority, 1-2 threads, yields between work units) for `zcache` jobs. Prevents shard rebuilds from starving `xargs -P 16` and vice versa.
- **Cross-shard JIT inlining DISABLED in v1** — inlining a callee from another shard creates stale-on-rebuild dependency. Cross-shard calls go through index lookup every time. Revisit only if benchmarked as hot.
- **Orphaned `.tmp.{pid}.{tid}` cleanup at startup** — worker job scans `images/` for `.tmp.*` files older than 1 minute and unlinks. Surfaced in `zcache verify`.
- **Log rotation built-in** — each log (`zshrs.log`, `zshrs-daemon.log`, `zshrs-recorder.log`) capped at 10 MB; rotates to `.1`. Configurable via `ZSHRS_LOG_MAX_BYTES` / `ZSHRS_LOG_MAX_ROTATIONS`.
- **Personality vs emulation scope** — personality mode (POSIX / vanilla zsh / turbocharged) is process-lifetime immutable; controls cache activation. `emulate -L` is per-function parser-flag scope only — fully supported, doesn't tear down cache. Two separate code paths.
- **POSIX mode gates entire layer off** — no `~/.zshrs/` created, no catalog open, no `zcache` builtins available, no worker cache jobs. Single `if !exec.posix_mode` check at each entry point.
- Worker pool hydrates `~/.zshrs/catalog.db` per-shard (`DELETE FROM entries WHERE plugin_id = '{shard}'` then INSERT). `entry_stats` survives via `ON CONFLICT DO UPDATE` keyed on `fq_name`. Hydration logs to `~/.zshrs/zshrs-daemon.log` via `tracing::info!` with `zcache_op` event shape.
- **`zcache <verb>` builtin family** — `info` / `jobs` / `clean [--all|shards|shard <name>|catalog|index|stats|log]` / `verify` / `compact`. All async write ops default to non-blocking; `--wait` for explicit sync. `zcache verify` runs `PRAGMA integrity_check` on demand (not every startup); on corruption prints `zcache clean catalog` recovery (then re-run `zshrs-recorder`) (entry_stats lost on recovery — single-dir trade).
- **Custom builtin namespace = `z*` prefix, no upstream-zsh clash** — `zcache` is the first; all future zshrs builtins follow same rule. Build-time anti-collision check.
- No bytecode in SQLite: bytecode lives only in the rkyv shards, and `plugin_cache.rs` drops the legacy `script_bytecode` table on open. The SQLite files are still opened for other data: `~/.zshrs/compsys.db` is the read-only completion mirror for `dbview` / SQL inspection, never read on the Tab path (`src/compsys/cache.rs:1-8`, `:31-42`). `~/.zshrs/plugins.db` is opened by `vm_helper.rs::open_plugin_cache` (`plugin_cache.rs::default_cache_path`). `.zwc` reading is live: `src/extensions/zwc.rs` loads stock-zsh `.zwc` files, used from `builtin.rs` and `parse.rs`.
- **Test:** install a plugin and re-run `zshrs-recorder`, call its function, verify it resolves through the rkyv path (index → shard mmap chain). dbview returns entry within ≤ shard rebuild duration. Edit one zpwr function, verify only `zpwr.rkyv` rebuilds. `zcache clean catalog` preserves `entry_stats` rows; `--no-stats` drops them. Run 60 concurrent writers of shard `plugin-foo`; verify the per-shard flock serializes them and the final shard is intact.
- **Acceptance:** full-corpus rebuild <30s clean; per-shard ~3-5s large / ~100-500ms small; cold shell launch <5ms; 60 parallel shells share <30 MB RSS attributable to images; lookup ~150-200ns; `zcache info` <100ms; `zcache verify` integrity scan in seconds.
- **Effort:** 8 days (sharded image format + hash-prefix naming + index.rkyv + perfect-hash + LRU shard-handle cache + generation counter + per-shard flock + per-shard worker hydrate + worker pool partition + log rotation + tmp cleanup + dbview integration + `zcache <verb>` builtin family + POSIX gating).

### G2a — zshrs-daemon spawn + lifecycle + IPC protocol
Per `AOT_DESIGN.md` §0x13 daemon architecture. G2 above ships the cache layer; G2a wires the daemon process around it.
- Separate `zshrs-daemon` binary (`daemon/bins/zshrs-daemon.rs`, workspace crate `daemon/`). Started by the user or a service manager (`examples/systemd/`, `examples/launchd/`, `examples/brew/`), never by the shell (`docs/DAEMON.md` "Daemon lifecycle").
- Connect-only clients: zshrs probes `~/.zshrs/daemon.sock` once at startup (`src/extensions/daemon_presence.rs`) — no handshake, no spawn. Daemon absent → degraded mode (source-interp, cache features no-op).
- Singleton: daemon takes `flock(LOCK_EX)` on `~/.zshrs/daemon.pid` at startup. Second instance sees lock held, exits.
- IPC: length-prefixed JSON over Unix domain socket. Versioned protocol; daemon and client negotiate at connect.
- POSIX-mode gating: zero daemon spawn under `--posix` / `emulate sh` / argv[0] basename `sh`/`dash`/`bash`. Single `if !exec.posix_mode` check at every cache entry point.
- Migrate G2's compile worker pool from per-client to daemon-only. Clients become read-only mmap consumers + IPC senders.
- Strict thin-client cap: per-client cache overhead <5 MB beyond zsh interpreter; ZERO cache-related background threads / polling loops / timers / SQLite handles in clients. Stats flush is event-driven (piggybacks on existing shell wake-ups).
- Crash recovery: clients never respawn the daemon. Shards + `catalog.db` are durable on disk; after the service manager (or user) restarts the daemon, clients reconnect and resume from cache. No state loss — everything reproducible from sources + on-disk shards.
- Logs to `~/.zshrs/zshrs-daemon.log` via `tracing::info!`. Built-in log rotation (10 MB cap per log, configurable via `ZSHRS_LOG_MAX_BYTES` / `ZSHRS_LOG_MAX_ROTATIONS`, `daemon/ticker.rs`).
- **Tests:** start daemon; N clients connect, none spawns; kill daemon mid-session, verify clients fall through to source-interp without crashing; restart daemon, verify clients reconnect; 100 parallel clients each connect; POSIX-mode invocation never probes the daemon, never creates `~/.zshrs/`.
- **Acceptance:** client connect <5ms; 100 parallel clients share <30 MB RSS attributable to images; daemon-down doesn't break shells; POSIX mode is daemon-free.
- **Effort:** 5 days (daemon binary + connect-only probe + flock singleton + IPC framing + POSIX gating + crash recovery + log rotation).

### G2b — Cross-shell coordination builtins (zconvey replacement)
Per `AOT_DESIGN.md` §0x13 "Daemon as cross-shell coordinator". Adds the `z*` builtin family for cross-shell registry, dispatch, notification, and pub/sub. All thin IPC wrappers — clients send JSON to daemon, daemon does the work.
- New IPC verbs in daemon protocol: `register`, `unregister`, `list_shells`, `dispatch`, `tag`, `notify`, `subscribe`, `unsubscribe`, `event_publish`.
- New `z*` builtins (all checked clean against upstream zsh):
  - `zls` — list active shells: id, pid, tty, cwd, tags, login_time
  - `zid` — print this shell's daemon-assigned ID
  - `zping` — daemon liveness + roundtrip latency probe
  - `ztag <name…>` / `zuntag <name…>` — self-tag this shell
  - `zsend <shell_id|--all|--tag <name>|--user <user>> <cmd>` — dispatch command/string
  - `znotify <shell_id> <message>` — status-line / OSC-9 notification (queues if target busy)
  - `zsubscribe <pattern>` / `zunsubscribe <pattern>` — pub/sub with `<shell_id_or_tag>.<event>` glob patterns
- Subscription model uses epoll-able fd in client's existing event loop — no dedicated thread.
- Shell registry maintained authoritatively by daemon via socket-connect enrollment (replaces filesystem-based registries used by zconvey).
- Plugins this replaces: zconvey, direnv (via daemon `chpwd` subscription set once globally), autoenv, zsh-history-substring-search shared state.
- **Tests:** spawn 5 zshrs clients, verify `zls` from any client lists all 5 with correct pid/tty; `zsend shell:2 'echo hi'` lands in shell 2; `ztag prod` then `zsend --tag prod 'echo'` reaches only tagged shells; `zsubscribe shell:1.commands` from shell 2 receives every command run in shell 1; `znotify` queues if target is busy; daemon-down → builtins fail gracefully with clear error.
- **Acceptance:** dispatch latency sub-ms (vs zconvey's ~1-prompt-cycle polling); zero per-client polling overhead; subscriptions work via existing event loop without spawning threads.
- **Effort:** 4 days (IPC verbs + builtin family + subscription pub/sub + tests).

### G2c — Cross-host federation (daemon-to-daemon over SSH multiplex)
Per `AOT_DESIGN.md` §0x13 — federation as forward-looking but architecturally enabled.
- Daemon-to-daemon protocol over SSH ControlMaster multiplexed connection. Local daemon discovers peer daemons via per-user known-hosts list (`~/.zshrs/peers`).
- New `zsend laptop:shell1 'open ~/notes.md'` — cross-host shell dispatch routes through local daemon → remote daemon → remote client.
- Cross-host history federation: remote daemon streams history events to local; local merges into history.db with origin tagging.
- Cross-host secret vending: local daemon holds secret; remote command requests via remote daemon → local daemon roundtrip; secret vended once to specific command, never written to disk on remote.
- `zls --all-hosts` lists shells across all federated daemons.
- **Tests:** SSH from laptop to server; spawn zshrs on server; verify local daemon sees remote shell in `zls --all-hosts`; dispatch command across hosts; secret vending works without disk write on remote; federation degrades gracefully when peer daemon unreachable.
- **Acceptance:** cross-host dispatch latency <100ms over LAN; secret vending leaves zero disk trace on remote; federation off by default (opt-in via `~/.zshrs/peers`).
- **Effort:** 7 days (peer discovery + daemon-to-daemon protocol + SSH multiplex integration + history federation + secret vending + auth/trust model).

### G2d — Session-persistent supervised jobs (zjob)
Per `AOT_DESIGN.md` §0x13 "Daemon as session-persistent supervisor".
- New `zjob` builtin family: `submit <cmd>`, `status [id|--all]`, `list`, `attach <id>`, `output <id>`, `kill <id>`.
- Daemon supervises jobs: spawns subprocess, captures stdout/stderr to log, retains job state for configurable retention period.
- Jobs survive originating-shell exit (the killer property). User opens shell B, runs `zjob submit 'long-build'`, exits shell B, opens shell C the next day, runs `zjob output <id>` — captured output displays.
- Output streaming: `zjob attach <id>` opens a stream of live job output to calling client (epoll-driven, no polling).
- IPC verbs in daemon protocol: `job_submit`, `job_status`, `job_list`, `job_attach`, `job_output`, `job_kill`.
- **Tests:** submit a 60s sleep + echo; exit submitting shell; new shell sees job in `zjob list`; `zjob output <id>` shows captured output after job completes; `zjob attach <id>` streams live; `zjob kill <id>` terminates cleanly; daemon survives client exit (job keeps running).
- **Acceptance:** jobs survive any/all client exit; output capture is loss-free; `zjob attach` latency <10ms.
- **Effort:** 6 days (job-supervisor logic in daemon + IPC verbs + builtin family + output capture/streaming + retention + tests).

### G3 — ZLE hooks + user widgets + completion firing
- `src/ported/zle/zle_main.rs` (hook system): implement `precmd`/`preexec`/`chpwd` hook dispatch — call the hook functions through the bytecode VM.
- `src/ported/zle/zle_main.rs` (user widget execution): when `bindkey '^X^E' my-widget` is bound, hitting that key invokes `my-widget` through the bytecode VM.
- `src/ported/zle/zle_main.rs` (prompt expansion): use the ported prompt expander; the gap is the wiring.
- `src/ported/zle/zle_hist.rs` (incremental search UI): real `Ctrl+R` impl in the native ZLE.
- Completion: when user types Tab in line editor, fire bound widget which calls the user's completion function (via `compdef name handler`); handler runs through bytecode VM, populates `executor.comp_matches`, line editor reads them.
- **Tests:** keypress simulation tests that bindkey a widget, simulate the keystroke, assert widget ran and side effects landed.
- **Acceptance:** tab-completing `git ` in zshrs produces the same matches as zsh (both backed by zsh-completions corpus).
- **Effort:** 4 days.

### G4 — zstyle, zmv, zparseopts, parameter flags
- `zstyle ':completion:*' completer ...`: builtin exists at `BUILTIN_ZSTYLE` (id 140) but the compsys-side reader needs to actually consult it.
- `zmv 'foo*.txt' 'bar*.txt'`: autoload from fpath, rebuild as bytecode.
- `zparseopts -A opts -- a b: c::`: builtin exists (id 147), tests needed.
- Parameter flags: `${(L)var}`, `${(U)var}`, `${(j: :)arr}`, `${(o)arr}`, `${(f)$(cmd)}`, `${(s. .)var}`, `${(@)arr}`, `${(k)hash}`, `${(v)hash}`, `${(P)var}`, `${(qqq)var}`.
- Each flag: extend `ZshrsHost::expand_param` with new `param_mod` constants. (The `BUILTIN_EXPAND_WORD_RUNTIME` fallback is gone — G8.)
- **Tests:** 50+ parameter-flag combinations.
- **Acceptance:** zpwr's parameter-flag-heavy lines work identically.
- **Effort:** 3 days.

### G5 — Glob qualifiers
- `*(.x)` (executable), `*(N)` (nullglob), `*(/)` (dirs), `*(@)` (symlinks), `*(.)` (regular files), `*(L+1024)` (size > 1KB), `*(mh-1)` (modified < 1 hour), `*(om[1,5])` (oldest 5).
- Modify `executor.expand_glob` to parse `(qualifier)` suffixes and filter results.
- The compile-time path: `compile_word` for Literal containing `(` after `*` routes to runtime fallback for now.
- **Tests:** 30+ qualifier tests against the local filesystem with controlled fixtures.
- **Acceptance:** every qualifier zsh manual lists works.
- **Effort:** 2 days.

### G6 — Job control parity — landed
- **Status:** landed 2026-10-18 (`28f7b6f766` ^Z suspend + `fg`/`bg` resume, `c6dd0c9929` superjobs for stops inside functions/`{ }`/loops, `3d882cb7b2` foreground `( … )` forked as the job). Behavioral pins: `tests/parity/jobs_parity.rs`.
- `jobs`, `fg`, `bg`, `wait`, `disown`, `kill %1` — builtins exist (ids 50-56).
- Background `&` / `&!`: the list carries `Z_ASYNC` (`src/extensions/compile_zsh.rs`, c:Src/parse.c:660-667).
- Job state transitions: SIGTSTP → Stopped, SIGCONT → Running, SIGCHLD → Done; `tcsetpgrp` for foreground swap.
- Signal forwarding: SIGINT to foreground job only, SIGWINCH to all jobs.
- **Tests:** spawn sleeping job, suspend with Ctrl+Z, list jobs, fg, verify state machine.
- **Acceptance:** `vim &; jobs; fg %1` works identically to zsh.
- **Effort:** 3 days.

### G7 — Signal traps (DEBUG, ERR, EXIT, ZERR)
- `trap 'echo bye' EXIT`: builtin id 100 exists; the dispatch is `Op::TrapSet(idx)` which the host's `trap_set` should record + fire on appropriate signal.
- DEBUG trap: fires before each command; emit `Op::TrapCheck` between commands in compile_simple.
- ERR trap: fires when last_status != 0; check after every SetStatus.
- EXIT trap: fires at script end; run before VM halts.
- ZERR trap: zsh-specific, fires on any error.
- **Tests:** each trap type with assertions that the trap fired exactly N times.
- **Acceptance:** zsh's `traps` testfile (W08traps) passes.
- **Effort:** 2 days.

### G8 — Kill `BUILTIN_EXPAND_WORD_RUNTIME` (the last residual dup) — landed
- **Status:** landed 2026-04-27. `BUILTIN_EXPAND_WORD_RUNTIME` (id 281) deleted in `593ad3432d`; `compile_word_runtime` went with `src/shell_compiler.rs` in `313481c810`. `rg 'BUILTIN_EXPAND_WORD_RUNTIME|compile_word_runtime' src bins daemon tests` returns nothing (2026-10-27). The plan below is kept as the record of what was removed.
- After G1 (arrays) and G4 (parameter flags) land, the only remaining users of `BUILTIN_EXPAND_WORD_RUNTIME` (the JSON-AST runtime fallback that delegates to tree-walker-era `expand_word_glob`) should be: mixed `$VAR + glob` literals, and any edge cases discovered during dogfood.
- Lower the mixed-literal case to ops: chain `Op::ExpandParam` + `Op::Glob` + `BUILTIN_ARRAY_JOIN` for literals containing both `$` triggers and glob chars.
- Audit all remaining call sites of `compile_word_runtime`. Any that exist after the lowering should either get a native op or be a documented "this case can't happen" assertion.
- Delete `BUILTIN_EXPAND_WORD_RUNTIME` (id 281), delete `compile_word_runtime` helper, delete the JSON-roundtrip serialization path entirely.
- `expand_word_glob` and `apply_var_modifier` either get inlined into the relevant host methods (where they're called by `Op::ExpandParam`) or deleted — they're tree-walker-era code that should not survive once nothing calls them.
- **Acceptance:** grep for `BUILTIN_EXPAND_WORD_RUNTIME` returns zero matches in src/. `cargo build` succeeds with zero warnings about unused functions in the expansion module.
- **Why this matters:** until G8 is done, zshrs has dual *implementations* of word expansion (native ops vs runtime fallback delegating to tree-walker code) even though dispatch is unified. That residual dup is the last debt from the rip-out. After G8, zshrs has literally one code path for every shell construct — no fallback, no alternative, no "if X then bytecode else tree walker" anywhere.
- **Effort:** 2 days after G1+G4 land.

**Phase G exit criteria:** maintainer's `.zshrc` + zpwr loads cleanly in zshrs with zero errors and zero behavioral differences from zsh. **Plus (met by G8, 2026-04-27):** `BUILTIN_EXPAND_WORD_RUNTIME` is deleted from the codebase. zshrs has literally one code path for every shell construct.

---

## Phase H — Verification (dogfood)

### H1 — `.zshrc` + zpwr load test
- New test file `tests/dogfood_zshrc.rs`: checks out (or symlinks) the real `.zshrc` and zpwr; runs zshrs with `-c 'source ~/.zshrc; echo READY'`; asserts `READY` is in stdout, exit 0.
- Captures stderr to a log file; any non-empty stderr fails the test.
- Run on every CI build.
- **Effort:** 1 day.

### H2 — zpwr 506+ subcommands functional test
- Generate a test case for each zpwr subcommand: `subcmd --help` produces non-empty output and exits 0.
- Bonus tests for the 50 most-used subcommands (from zsh history frequency): verify their actual functionality.
- **Effort:** 3 days.

### H3 — Completion-corpus smoke test (47,394 zsh-more-completions files as of 2026-10-27)
- Iterate every `_*` completion file; ensure it loads (autoload + bytecode compile) without errors.
- For 100 most-common commands, verify `compdef` registered the handler.
- **Effort:** 1 day.

### H4 — `dogfood -live` mode
- 30-day rotation: alternate days using `chsh -s zshrs` and `chsh -s zsh`.
- Log every error hit in zshrs to a known file.
- Each error becomes a regression test.
- **Effort:** 30 days calendar (parallel with other phases).

**Phase H exit criteria:** 14 days of zshrs as login shell with zero panics, zero hangs, zero command-output differences vs zsh.

---

## Phase I — Storage durability (endgame schema)

### I1 — Bytecode chunk format versioning
- **Mechanism landed (rkyv, not SQLite).** No bytecode is stored in SQLite: `plugin_cache.rs` drops the legacy `script_bytecode` table on open, and `compsys.db` is a read-only `dbview` mirror (`src/compsys/cache.rs:1-8`). Bytecode lives in the rkyv shards `scripts.rkyv` and `autoloads.rkyv`, each with a header `{ magic, format_version, zshrs_version, pointer_width, built_at_secs }` (`SHARD_FORMAT_VERSION` = 5 in `script_cache.rs`, 7 in `autoload_cache.rs`).
- On read, `header_ok` rejects a shard whose magic, `format_version`, `zshrs_version` or pointer width differs from the running build, so every lookup misses and the source is recompiled. On write, such a shard is replaced by `fresh_shard()` (`script_cache.rs:340-349`, `autoload_cache.rs:361-371`).
- **Test (not written):** write a `scripts.rkyv` / `autoloads.rkyv` with an older `format_version`, then verify zshrs serves no entry from it, recompiles from source and rewrites the shard with the current header. No test currently pins `header_ok` rejection.
- **Effort:** 1 day.

### I2 — SQLite schema migrations
- Each cache db (`catalog.db`, `history.db`, `cache.db` under `~/.zshrs/`) gets a `schema_version` table.
- On open: check version; if older, run migration scripts; if newer, fail loudly (downgrade is unsafe).
- **Test:** open a v0 db with v1 zshrs, assert clean migration.
- **Effort:** 1 day.

### I3 — MSRV + Cargo.lock policy
- Pin MSRV in Cargo.toml `rust-version = "1.75"` (or whatever current minimum is).
- Commit `Cargo.lock`.
- CI matrix: build against MSRV + stable + beta to catch toolchain regressions early.
- **Effort:** 0.5 day.

### I4 — Dependency audit
- `cargo audit` in CI.
- Review every dep in Cargo.toml: who maintains it, last release, alternatives.
- Vendorable test: `cargo vendor && cargo build --frozen --offline` must succeed.
- **Effort:** 1 day.

---

## Phase J — Test invariant ratchet (parallel with G)

Each G item ships with its own tests. Phase J is the additional sweep beyond per-phase tests.

### J1 — setopt flag matrix
- Every zsh option (`AUTO_CD`, `EXTENDED_GLOB`, `NULL_GLOB`, `HIST_IGNORE_DUPS`, etc.) has a test that toggles it on, performs an action, asserts behavior. Same with off.

### J2 — Glob qualifier exhaustive
- Every qualifier in `info zsh "Glob Qualifiers"` has a test.

### J3 — Parameter flag exhaustive
- Every flag in `info zsh "Parameter Expansion Flags"` has a test.

### J4 — Redirect form exhaustive
- All 9 redirect ops × all 4 here-doc/string variants × file/fd target = ~50 tests.

### J5 — POSIX conformance subset
- Run a subset of Open POSIX Test Suite shell tests; track pass rate as a metric over time.

**Target:** 500+ behavioral tests by phase J completion.

---

## Phase M — Performance proof

### M1 — Hyperfine harness in CI
- New crate `bench/` with hyperfine-driven scripts comparing zshrs vs zsh vs bash on:
  - Cold startup (`time $shell -c true`)
  - Warm startup (`time $shell -c 'source ~/.zshrc; true'`)
  - Pipeline (`echo $(seq 100) | tr ' ' '\n' | sort | uniq | wc -l`)
  - 100 builtin calls (`for i in {1..100}; do echo $i; done`)
  - Glob expansion (`echo **/*.rs` over the repo)
  - compinit (`time autoload -Uz compinit; compinit`)
- **Effort:** 1 day.

### M2 — Publish numbers
- Run on actual machine (M-series Mac, real .zshrc).
- Update README perf table with hyperfine output.
- Retire any unverified claim in README/RFC; replace with measured numbers.
- **Effort:** 1 day.

### M3 — Regression alarm
- CI runs the bench on every merge; any regression > 10% fails CI.
- **Effort:** 0.5 day.

**Phase M exit criteria:** README's performance claims are reproducible from a single command. No vapor numbers.

---

## Phase K — Cross-platform CI

### K1 — GitHub Actions matrix
- macOS aarch64 (daily driver)
- macOS x86_64 (Intel Macs still in use)
- Linux x86_64 (servers)
- Linux aarch64 (Graviton, RPi)
- FreeBSD x86_64 (optional, low priority)
- **Effort:** 1 day.

### K2 — Per-platform behavioral tests
- Some tests are macOS-specific (`/usr/bin/true` exists, `/bin/true` doesn't).
- Some are Linux-specific (procfs, different default PATH).
- Tests gated with `#[cfg(target_os = ...)]` where needed.
- **Effort:** 1 day.

---

## Phase L — Synthesis features (after compat floor solid)

These features absorb the best of other shells. None ship before Phase G is done.

### L1 — Fish-style syntax highlighting in ZLE
- Already partially in `fish/` subtree; finish wiring into the native ZLE highlighter.

### L2 — Fish-style autosuggestions
- History-based; SQLite history backend already exists; just needs the inline-render hook.

### L3 — Atuin-style history search
- `history.db` is SQLite + FTS5; expose `Ctrl+R` that does FTS query, not substring.

### L4 — nushell-style structured data via stryke
- `ls | from nu | where size > 1MB` — pipe is JSON between zshrs and stryke.

### L5 — Abbreviations (zsh-abbr / fish abbr)
- Type `gco`, space → expands to `git checkout`.
- ZLE-level feature; partially in `fish/abbrs.rs`.

---

## Phase N — Documentation honesty pass

### N1 — README rewrite
- Every claim has a test or benchmark backing it.
- Drop "100% compiled" until it's literally true (Phase G7 done).
- Drop "100x" perf numbers until M2 publishes real numbers.
- Add "what works today / what's coming / what zsh does that zshrs doesn't yet."

### N2 — RFC update
- Same honesty pass.
- Update phase status from aspirational to measured.

### N3 — Architecture doc
- New `docs/ARCHITECTURE.md` covering: VM dispatch, ShellHost callbacks, rkyv cache + SQLite mirrors, worker pool, fork-per-stage pipelines.
- Diagram-heavy, code-references-by-line-number for navigability.

### N4 — Plugin author guide
- For people writing zshrs-aware plugins: AOP intercept usage, async/await/pmap, dbview, profile builtin.

### N5 — Migration guide for zsh users
- Side-by-side: "this zsh idiom works the same / works differently / doesn't work yet."

---

## Phase O — Release v1.0

### O1 — Packaging
- Homebrew formula
- AUR PKGBUILD
- Nixpkg
- Debian deb (lower priority)

### O2 — Stable API guarantee
- v1.0 means: bytecode format frozen, builtin dispatch table frozen, SQLite schemas frozen with migration support.
- Future breaking changes go in v2.0 with migration path.
- Semver discipline going forward.

### O3 — Bench data + announcement
- Blog post / GitHub release with M2 numbers.
- Cross-link from zpwr README.

### O4 — Adoption metrics
- Track crates.io downloads, brew formula installs, GitHub stars.
- Not vanity — early signal of who else is finding it useful.

---

## Realistic timeline

| Phase | Items | Wall time | Calendar (part-time) |
|-------|-------|-----------|----------------------|
| G | G0–G7 | 17 days work | 4–8 weeks |
| H | dogfood | 30 days calendar | parallel with G/I |
| I | storage durability | 4 days | parallel with G |
| J | test ratchet | continuous | every PR |
| M | perf bench | 2 days + ongoing | week 6+ |
| K | cross-platform | 2 days | week 8+ |
| L | synthesis | 2-3 weeks | post-Phase G |
| N | docs | 5 days | week 10+ |
| O | release v1.0 | 1 week | month 3-4 |

**Total:** ~3-4 months calendar to v1.0 endgame. Honest estimate assuming part-time work on top of day job. Full-time would be ~6 weeks.

---

## Today actions

1. **G0 audit + lint** — sweep stops the friendly-output regression class permanently. 2 hours.
2. **Phase G1 (arrays) — landed:** argv splice ships in the pinned fusevm series; zshrs tracks **0.26.7** (`fusevm` in root `Cargo.toml`, features `jit`, `jit-disk-cache`, `aot`, `ffi`, `rkyv-archive`). Next moves are assoc-edge cases (see `TODO.md`).
3. **Decide priority of Phase H1 dogfood** — can be started in parallel with remaining assoc tail work by using a stripped-down `.zshrc` that avoids the long tail of zpwr-only constructs. Surfaces more bugs sooner.

---

## What remains aspirational until measured

These claims appear in README/RFC but are not yet backed by reproducible benchmarks. They get retired or substantiated in Phase M.

- "100x warm start speedup"
- "2000-5000x cat fork avoidance"
- "7x CI/CD pipeline speedup"
- "100-400x script startup vs bash/zsh"
- "48 coreutils builtins" (verified — `EXT_BUILTIN_NAMES` entries that GNU coreutils also installs, listed in README)

Phase M produces measured replacements. Until then, the README's "performance" section reads as architecture description, not benchmark report.

### Counts verified against the tree (2026-10-27, `7467218ed8`)

Every row was recounted on 2026-10-27 at `7467218ed8` with the command in the "How" column. Git pathspecs (`'dir/*.rs'`) match recursively.

| Claim | Measured | How |
|---|---|---|
| Builtins | **259** (157 + 102, disjoint) | Distinct names in `BUILTIN(` / `BIN_PREFIX(` entries of the `BUILTINS` vec in `src/ported/builtin.rs`, comments stripped (160 entries; `fg`, `kill`, `suspend` appear twice), plus the 102 names in `EXT_BUILTIN_NAMES` in `src/extensions/ext_builtins.rs`. `comm -12` of the two sorted lists finds 0 names in common. |
| fusevm opcodes | **235** | Variants of `pub enum Op` in `~/.cargo/registry/src/*/fusevm-0.26.7/src/op.rs` (the version in `Cargo.lock`), comments and attributes stripped |
| ZLE widgets | **193** | String entries of `IWIDGET_NAMES` in `src/ported/zle/zle_bindings.rs` |
| Workspace Rust | **1,019 files / 1,082,434 lines** | `git ls-files '*.rs' \| wc -l`; `git ls-files -z '*.rs' \| xargs -0 cat \| wc -l` |
| `src/ported/` | **106 files** (40 at top level) | `git ls-files 'src/ported/*.rs' \| wc -l` |
| `src/extensions/` | **138 files** | `git ls-files 'src/extensions/*.rs' \| wc -l` |
| compsys mirror | **1,249 files** (250 `.rs`, 994 extensionless completion functions) | `git ls-files 'src/compsys/ported/' \| wc -l` |
| p10k engine | **28 files / 25,719 lines** | `git ls-files 'src/extensions/p10k/*.rs' \| wc -l`; same list piped to `xargs -0 cat \| wc -l` |
| demo scripts | **375** | `git ls-files 'examples/demos/*.zsh' \| wc -l` |
| assertion verbs | **14** | `rg -o '"zassert_\w+"' src/extensions/ext_builtins.rs \| sort -u \| wc -l` |
| behavioral pins | **174** | `rg -c '#\[test\]' tests/no_tree_walker_dispatch.rs` |

Numbers in README, the docs site, and `docs/INVENTIONS.md` are kept to these.
