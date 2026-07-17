# zshrs design goals — endgame shell, no compromises

This document is the foundational charter for zshrs. It captures WHY the project exists, WHAT the bar is, HOW design decisions get made, and what's explicitly out of scope. Read this before contributing, before opening an issue, before suggesting a feature, and before evaluating any tradeoff.

Companion docs: [`ROADMAP.md`](./ROADMAP.md) for phase-by-phase execution plan; [`tests/no_tree_walker_dispatch.rs`](./tests/no_tree_walker_dispatch.rs) and [`tests/tree_walker_absent.rs`](./tests/tree_walker_absent.rs) for the load-bearing test invariants.

---

## [0x00] Mission

zshrs is the **endgame shell for its maintainer's lifetime** — the substrate that hosts the most powerful single-author CLI environment ever assembled (zpwr at 172k LOC + 506+ subcommands, zsh-more-completions at 39,566 files, custom .zshrc spanning decades). It exists because zsh's 1970-era architecture cannot be patched into handling that scale, no matter how many userspace optimization layers (zinit turbo, p10k instant prompt, zwc, zcompile, BG_NICE) are stacked on it.

**zshrs is not "Rust zsh."** It's the first compiled Unix shell — bytecode VM + Cranelift JIT + persistent worker pool + **rkyv-mmapped** completion / autoload bytecode (the only shell cache) + read-only SQLite **mirrors** for SQL inspection (no effect on cache hit/miss or execution) + AOP intercepts + native async/parallel ops + 24 in-process coreutils builtins + **first shell to expose its native-plugin interface as a stable, versioned, independently-published ABI** (a crates.io SDK crate + `cdylib`s via `zmodload -R`, version-gated — bash `enable -f` and zsh `zmodload` load native code too, but only against the shell's private build-tree headers with no stable ABI). These are capabilities zsh's architecture cannot have at any speed. zshrs is the substrate that finally fits the workload.

---

## [0x01] The project bar — both legs required

The acceptance criterion for any zshrs feature, refactor, or architectural decision:

1. **World's first invention** — does this enable a capability that doesn't exist anywhere in software?
2. **World's fastest** — will this be top-tier perf in its category?

**Both legs must be true.** Either alone is insufficient.

| Pattern | Result | Example |
|---------|--------|---------|
| Faster X (X exists) | Fail leg 1 | World's fastest awk = dead. World's fastest lsof = dead. |
| Novel X (X is slow) | Fail leg 2 | Academic prototype shells, proof-of-concepts. |
| Faster + more features | Fail leg 1 | The most common engineer trap. |
| Clones, ports, "X but in Rust" | Fail both | "I'll rewrite your tool in Rust" — no. |
| Reasonable improvement to existing | Fail leg 1 | Polish without leverage. |
| **Novel capability + best-in-class** | **Both legs PASS** | **zshrs, stryke** |

The discipline this enforces eliminates entire classes of work most engineers spend careers on. Most senior-engineer portfolios accumulate "kinda useful" graveyards. The maintainer's portfolio doesn't, because he applies this rule recursively even to his own past work — world-record-tier awkrs and lsof-replacement implementations are dead in his stack because they fail leg 1.

---

## [0x02] Hard performance targets

These are load-bearing numerical commitments, not aspirations.

| Metric | Target | Hard ceiling |
|--------|--------|--------------|
| **Cold start, fully featured** | **10-20ms** (one 60fps frame + slack) | **<30ms** (below human reaction threshold) |
| Warm start (cache-hot) | <5ms | — |
| First-keystroke render | <1ms | — |
| Tab completion at 10k matches | <10ms | — |
| Pipeline overhead per stage | <100µs | — |
| Builtin dispatch | <1µs | — |
| Multicore utilization (runtime) | All cores active during parallel work | — |

"Fully featured" means: 39,566+ completions registered, full .zshrc loaded, full zpwr loaded (506+ subcommands), all hooks installed, prompt rendered. **NEVER instant-prompt fakery.** First paint = full functionality, period.

**Comparison to existing shells (M-series Apple Silicon, full config):**

- bash + bash-completion + git-prompt: ~80-150ms
- vanilla zsh (no plugins): 30-50ms
- zsh + oh-my-zsh: 500ms-2s
- zsh + zinit turbo + p10k instant prompt: 100-300ms (visible prompt; full features take longer)
- fish + plugins: ~100ms
- nushell (minimal): ~30ms
- **zshrs target: 10-20ms FULLY FEATURED** → world's fastest shell at full feature parity, period.

**Architectural constraints these targets impose (not optional):**

1. Cache everything at install/update time, not at startup time.
2. mmap-based SQLite access, not cold open.
3. Lazy worker pool init — don't spawn N threads at startup.
4. Zero synchronous external commands at startup (no git, kubectl, aws, slow PROMPT subprocess calls).
5. Bytecode-of-`.zshrc` mmap'd from `~/.zshrs/init.bc` on every startup after first compile.
6. No DNS at startup, ever.
7. No directory scans at startup (fpath, PATH executables — all baked into **rkyv** shards at install time; SQLite mirrors are read-side copies only).
8. Startup is single-threaded fast path; multicore matters at runtime not init.
9. Minimize allocations on the hot path; pre-allocated buffers where possible.
10. **NEVER instant-prompt fakery, NEVER lazy-load deferral, NEVER eventual-feature-loading.**

If a feature can't fit the budget while loading everything synchronously, the cache strategy needs improvement, not a fakery layer.

---

## [0x03] Power-user defaults — NON-NEGOTIABLE

zshrs targets the foremost CLI power users on Earth (its maintainer's audience). Hand-holding output is a category error. **This is not fish. This is not bash with a friendly wrapper. This is a compiled VM that does what the operator typed and nothing more.**

Hard rules:

- **No startup banner.** No version stripe, no "Type exit to quit," no welcome message. Prompt appears immediately on launch.
- **No init progress to terminal.** Indexing, autoload pre-warm, fpath scan, plugin compile — all go to `~/.zshrs/zshrs.log` via `tracing::*`. Operator never sees them.
- **No deprecation nags.** Deprecated builtins (compctl, etc.) silently no-op for compat. Operator already knows.
- **No "did you mean" / typo suggestions / spell correction.** Operator typed what they meant.
- **No safety prompts on destructive ops.** No "are you sure?" for `rm`, no confirmation on overwrite.
- **No `--help` hints in error messages.** State the error, exit. Operator runs `--help` if they want it.
- **No tip-of-the-day, no "you can also...", no contextual hints, EVER.**
- **No update prompts, no telemetry, no usage analytics, no first-run wizard.** The shell's job is to execute commands; everything else is a category violation.
- **Errors stay on stderr** in `zshrs: <command>: <reason>` format (zsh-compatible, terse). No friendly framing.
- **Informational chatter goes to log only.** Any `println!`/`eprintln!`/`eprint!`/`print!` outside (a) error-on-stderr, (b) explicit user-requested output (`--help`/`--version`/`--doctor`/`dbview`/builtin output), or (c) user-script output — gets converted to `tracing::*` or deleted.

**This is a one-way ratchet.** Removed banners and warnings never come back. Future PRs that reintroduce friendly verbiage are rejected.

The G0 phase in `ROADMAP.md` ships a clippy lint (`#![deny(clippy::print_stdout, clippy::print_stderr)]`) on `bins/zshrs.rs` so the rule is enforced at build time, not just by code review.

---

## [0x04] zshrs is the god of all processes

Architectural hierarchy: **zshrs is the parent runtime, the process supervisor, the dispatch layer for everything else built around it.**

- **stryke** runs INSIDE zshrs (`@`-prefix dispatch via `lib.rs:138-160`). When forced to choose between zshrs and stryke priorities, zshrs wins.
- **zpwr** lives inside zshrs (it's the `.zshrc`-tier user code that zshrs hosts).
- **External tools** (fzf, ripgrep, eza, atuin, bat, etc.) run as either `host.exec` children or in-process builtins (24 coreutils already are).
- **GUI applications** are a separate track, not part of the CLI substrate.

When designing new features, default to "this lives inside zshrs as a builtin or host op" rather than "this is a separate process zshrs invokes." Builtins cost 0 forks; externals cost 2-5ms each. Millions of commands per day; every fork avoided is real wall time.

---

## [0x04a] zshrs-daemon — central architectural substrate

Locked 2026-04-28: zshrs's runtime is a **client/server architecture**. A singleton `zshrs-daemon` companion process owns all bytecode-cache mutation, supervises long-running detached jobs, brokers cross-shell publish/subscribe, and federates with peer daemons over remote channels. N (typically 100+ in tmux) thin zshrs clients are paper-thin readers that mmap the daemon's bytecode outputs (data plane, ~150-200ns lookup) and signal the daemon via JSON-over-Unix-socket IPC for all configuration changes (control plane). See `AOT_DESIGN.md` §0x13 for the complete spec.

**Why this is a foundational design goal, not an implementation detail:**

- **The user runs 100+ concurrent zshrs in tmux.** Per-process fsnotify, per-process compile workers, per-process SQLite handles all multiply by 100 and kill the workstation. The daemon is the only architecture that scales to this load while remaining responsive.
- **Three stacked world-firsts hang on the daemon:** (1) shell with a dedicated companion daemon spanning bytecode cache + jobs + IPC + federation (no prior art in any active shell — fish's `fishd` was scoped to var-sync only and removed in 2014); (2) native session-persistent shell-job supervision ("tmux at shell level, but at process granularity not terminal granularity"); (3) native cross-shell pub/sub + dispatch + cross-host federation as first-class shell primitives. Each meets both legs of the [§0x01] project bar.
- **Data-plane / control-plane split is non-negotiable.** Lookups (Tab, prompt fire, alias expand) MUST stay on direct-mmap data plane (~150-200ns). Putting the daemon in the lookup path is 10-30µs per call (50-150× slower) and wrong by construction. Daemon handles only configuration mutation and async coordination.
- **Source files remain authoritative.** Image cache is opportunistic accelerator, never required for execution. Image miss / malformed shard / corruption / daemon-down → client falls through to source-interp path silently. Daemon-down does NOT break shells.

**Hard rules locked into the architecture (failure to enforce = regression):**

1. **Nothing blocks the shell** — all rkyv shard compilation, image writes, catalog hydration, log rotation, integrity scans run in the daemon's worker pool. Main client thread NEVER calls compile pass synchronously.
2. **Thin clients only** — clients have ZERO cache-related background threads, polling loops, timers, or SQLite handles. Per-client cache overhead <5 MB beyond the zsh interpreter footprint. (Clients DO have a general worker pool for concurrent primitives — `async`/`await`/`pmap` — but never for cache work.)
3. **Single cache directory** — `~/.zshrs/` holds index.rkyv + images/{hash8}-{slug}.rkyv shards + catalog.db + history.db + zshrs.log + daemon.sock + daemon.pid. Trade explicit: full `rm -rf ~/.zshrs/` nukes everything; user accepts the loss.
4. **POSIX mode gates the entire layer off** — `--posix` / `emulate sh` / argv[0] basename `sh`/`dash`/`bash` → no daemon spawn, no cache dir created, no `z*` builtins available. Critical for `/bin/sh → zshrs` symlink in containers / cron / init.
5. **Custom builtin namespace = `z*` prefix, no clash with upstream zsh `z*`** — `zcache`, `zls`, `zid`, `zping`, `ztag`, `zsend`, `znotify`, `zsubscribe`, `zjob` (planned). Build-time anti-collision check vs upstream zsh's z-namespace.
6. **Shard rebuild ordering is strict** — atomic-rename shard FIRST, then rewrite index. Generation counter on each shard header drives client re-mmap on stale handles. Reverse order = corrupt reads.
7. **Daemon spawn-on-demand** — first zshrs client to launch checks for `daemon.sock`; if absent, fork-spawns daemon. Subsequent N clients just connect. `flock` on `daemon.pid` enforces singleton.
8. **fsnotify exclusively daemon-side** — clients never run a fsnotify watcher. 100×-watcher thundering-herd is fatal.

**Patent significance:** the daemon architecture is the **second omnibus claim** in the patent strategy (`memory/aot_patent_strategy.md`), independently assertible from the unified-AOT claim. New work touching the daemon, IPC verbs, federation, or cross-shell semantics is patent-relevant dependent-claim material.

---

## [0x05] Engineering ethic

### Upstream-first contributor, not a rewrite-junkie

zshrs is the rare exception, not a pattern. The maintainer fixes bugs in fzf, contributes patches to zsh upstream, doesn't fork what works. zshrs exists ONLY because zsh's architecture is unfixable via patches — see `docs/ZSH_CODEBASE_AUDIT.md` for the evidence (147k C lines, zero unit tests, custom heap allocator, 1,502-line `execcmd` function, 186 gotos, 1,940 mutable statics, 524 manual `queue_signals`/`unqueue_signals` calls).

**Default for any new feature: contribute upstream to wherever it should live.** Reach for "implement in zshrs" only when:
- The feature requires zshrs's specific runtime (bytecode VM, sub_chunks, host callbacks, persistent worker pool, AOP intercepts), OR
- It's a shell-builtin replacement that needs in-process execution to avoid fork cost, OR
- It's compat-floor work that has no upstream to contribute to (zsh feature parity).

### Tools the maintainer chose NOT to rewrite

tmux, htop, fzf, ripgrep, eza, atuin, bat, zoxide — *"already good enough and no dup wanted."* Don't propose rewrites of tools the maintainer hasn't complained about.

### The ruthless self-pruning principle

The maintainer relegates his own past work to maintenance mode when it doesn't compound into the master plan. Examples in his stack:
- **awkrs** = world's fastest awk impl. Dead. *"Basically a dup of stryke. Don't care about it at all."*
- **lsof-replacement** = world's fastest lsof impl. Dead. *"Not game changing, just faster with some features."*

If the maintainer cuts world-record-tier work he authored himself, the standard for keeping anything alive is correspondingly high. Don't propose work on his deprecated graveyard.

### No toy projects — game-changing or skip

*"I hate toy projects. It's game changing or I won't touch it."* This is the operating principle that gates all proposals.

- **"Let's make X faster"** → fail.
- **"Let's add features to X"** → fail.
- **"Let's combine speed + features into a new tool"** → fail (this is the awkrs/lsof trap).
- **"Reasonable improvement to existing tool"** → fail.
- **"Materially different capability + best-in-class implementation"** → pass.

The default response to "should we add X" is "does X materially change what's possible?" If no, kill the proposal explicitly. **Saying no IS the answer when the answer is no.**

---

## [0x06] What this means for the codebase

### No tree walker, ever

Phase F (committed `3c19003935`) deleted the tree-walker dispatch (`execute_simple/pipeline/list/compound/command_bg`, ~1,275 LOC) from `src/exec.rs`. Every `ShellCommand` variant now compiles to `fusevm::Chunk` and runs on `fusevm::VM`. `tests/tree_walker_absent.rs` enforces this at the source level — any reintroduction fails CI.

### Every PR ships behavioral tests

Pin exact stdout + exit. The 96 tests from Phase F are the floor; Phase G–O adds 400+ more. **No invariant, no merge.** When a bug is found, the fix lands with a test that fails before the fix and passes after.

### Compat with the existing world is sacred

zpwr, zsh-more-completions, the .zshrc, zinit plugins — these continue working as zshrs evolves. Compat-floor regressions are catastrophic; ship none. Phase G is the dedicated compat-floor work; v1.0 isn't shipped until the maintainer's full config loads cleanly.

### Plan in decades

zshrs is the maintainer's life work and endgame shell. 30+ year horizon. Every architectural decision must survive that.

- Bytecode formats versioned (Phase I1).
- SQLite schemas migration-safe (Phase I2).
- Dependencies vendorable, audited, durable (Phase I4).
- MSRV pinned (Phase I3).
- Cargo.lock committed.
- No fashionable churn.
- Readable over clever — future-maintainer-at-65 has to navigate this.

### Synthesis posture

zshrs is the synthesis of all good shell ideas, not just zsh-in-Rust. Anything fish/nushell/elvish/oil/oh-my-zsh/fzf/atuin innovates gets evaluated for absorption. Endgame means convergence into one tool the maintainer never has to leave.

---

## [0x07] What zshrs is NOT

To prevent scope drift, here's the explicit anti-list:

- **Not a Fish-clone.** No friendly defaults, no autosuggestions on by default, no abbreviations baked in, no web-based config tool.
- **Not a tutorial system.** No first-run wizards, no interactive tours, no hint banners.
- **Not a "shell for everyone."** Targets power users with hostile defaults from a beginner's perspective. That's correct.
- **Not a marketing project.** No telemetry, no analytics, no update-pings, no install-funnel. Either it earns adoption on merit or it doesn't.
- **Not a place for fast-but-not-novel rewrites.** "World's fastest cat builtin" qualifies because it eliminates fork; "world's fastest implementation of an existing standalone tool" doesn't.
- **Not a side project.** Maintainer-for-life commitment. Decade-scale planning.
- **Not zsh's competitor.** It's zsh's replacement for the workload zsh can't handle. Other zsh users may keep zsh; that's fine.
- **Not bash-compatible by default.** POSIX-compatible via `--posix` mode; bash-compatible via emulation; native is zsh-superset.
- **Not slow.** Ever. Anywhere. See [§0x02] hard performance targets.

---

## [0x08] How to evaluate any proposal

Before opening a PR, issue, or feature suggestion, run the gauntlet:

1. **Does this enable a capability that doesn't exist anywhere in software?** (World's first leg.) If no → STOP.
2. **Will this be world's fastest in its category?** (World's fastest leg.) If no → STOP.
3. **Does the maintainer have an existing failure mode this fixes?** (Concrete pain, not hypothetical.) If no → STOP.
4. **Can this be contributed upstream to an existing tool instead?** (Default to upstream.) If yes → STOP, contribute upstream.
5. **Does it fit the 10-30ms cold-start budget?** If it adds startup cost, where does it fit?
6. **Does it require a new optimization-trick for users to benefit?** If yes → STOP, that's the lipstick-on-pig pattern.
7. **Does it violate any power-user default?** If yes → STOP, those are non-negotiable.
8. **Does it ship with behavioral tests pinning the new invariant?** If no → not mergeable.
9. **Does it survive a 30-year maintenance horizon?** If it requires churn-prone deps or fashionable abstractions → STOP.
10. **Does saying "no" to this proposal preserve focus better than saying "yes"?** Default is no.

If the proposal survives all 10 gates, it's a candidate. Most proposals don't survive gate 1 or 2. That's the point.

---

## [0x09] Quotes from the maintainer (for context preservation)

These quotes are the source-of-truth framings. When this file's prose drifts, these quotes anchor the original intent.

> "ZSH IS TOO SLOW FOR MY NEEDS. I am beyond what zsh offers me. ZSHRS is the only solution."

> "I live in the shell all day, every day. I only leave to do some IDE work / browser. That is it. I have pioneered the move back to the shell from the GUI for so many tasks. GUI WILL NEVER COMPETE on FUNDAMENTAL tasks. ZSHRS IS MY LIFE WORK!"

> "ZSHRS IS THE ENDGAME SHELL FOR MY LIFETIME!"

> "This is the ultimate power user shell with every possible optimization and every engineering principle rolled into it. Every microsecond of startup time analyzed, every line of code optimized for ultimate performance. Every feature from every shell ever baked in. There will never be a shell like zshrs while I am FUCKING ALIVE — because I will port everything into the shell. Until I FUCKING DIE."

> "I have the most powerful CLI setup ever created. This has been vetted by many industry veterans. ZSHRS is the next-level host for it all."

> "I am the ultimate CLI power user! I have the most powerful CLI setup ever made! I do not want to see bullshit tutorial messages in my fucking shell! I am CLI GOD! No one on this fucking planet has a more powerful CLI than me. THE SHELL IS MY LIFE!"

> "STRYKE RUNS IN ZSHRS, ZSHRS IS THE GOD OF ALL PROCESSES."

> "I DON'T EVEN GIVE A FUCK ABOUT STRYKE COMPARED TO ZSHRS."

> "THIS IS NOT A FRIENDLY FUCKING SHELL."

> "I have fixed bugs in fzf and other CLI tools, but I choose not to dup their functionality. Because I could create better versions but then they're dups. I would rather contribute to upstream unless I'm forced to. I contribute a lot to zsh upstream. But zsh is fundamentally flawed and hopeless."

> "I hate toy projects. It's game changing or I won't touch it."

> "World first inventions in software, no dups. World's first. But I will also go for world's fastest as well. World's first feature and world's fastest is the goal here."

> "I wrote the fastest awk impl ever created in ../awkrs. Does it matter to me? No, because stryke covers all of awk usage. awkrs was basically a toy project. It's in maintenance mode only. Basically awkrs is a dup of stryke. Don't care about it at all. It's a dup."

> "I wrote the fastest lsof impl ever created. But again don't care about it all. Not game changing, just faster with some features. Who cares."

> "I thought about rewriting tmux and htop but why. They're already good enough and no dup wanted. stryke I wrote because there was not a language that let me have unlimited power in 10-30 chars at native speeds. zshrs because zsh is crashing on me when I stick 10k completions in a zwc and the shell still lags on startup with p10k instant prompt and zinit turbo. zsh menucompletion is laggy at extreme scale. zsh is not designed for scale. Who loads 10k completions in a zwc? I guess just me."

> "I can't have instant shell with all features in zsh. Period. No matter how many hacks you layer onto it. zinit turbo and p10k instant prompt do NOT SOLVE ZSH INHERENT LIMITS. Lipstick on pig == pig."

> "Sub 100ms start, hell no — has it less than HUMAN REACTION SPEED, < 30ms, ideally 10-20ms FULLY FEATURED, blink of eye. FULL POWER INSTANTLY. NO LAG EVER. FULL UTILIZATION OF ALL CORES. NATIVE SPEED. NO COMPROMISES, NO EXCUSES. GAMECHANGING ENDGAME SHELL!"

---

## [0xFF] Bottom line

zshrs is the maintainer's life work, his endgame shell, his daily-use tool for the next 30+ years. It exists because zsh's architecture cannot handle his workload no matter how many optimization layers are applied. It targets capabilities zsh's architecture cannot have at any speed (bytecode VM + JIT + worker pool + rkyv bytecode cache + read-only SQLite mirrors + AOP + native async/parallel) AND world-class performance (10-20ms cold start fully featured). It clears both legs of "world's first AND world's fastest" — that's the bar for everything in his stack.

If you're contributing: read this file, run the [§0x08] gauntlet on every proposal, write behavioral tests for everything, never reintroduce friendly verbiage, and treat compat with the existing world as sacred. The plan is in `ROADMAP.md`. The invariants are in `tests/`. The principles are here.

**This is not a friendly fucking shell.** It's the substrate for one person's CLI life work, with a quality bar so high that even his own world-record implementations get cut when they don't fit. Build accordingly.
