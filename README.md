```
 ███████╗███████╗██╗  ██╗██████╗ ███████╗
 ╚══███╔╝██╔════╝██║  ██║██╔══██╗██╔════╝
   ███╔╝ ███████╗███████║██████╔╝███████╗
  ███╔╝  ╚════██║██╔══██║██╔══██╗╚════█��║
 ███████╗███████║██║  ██║██║  ██║███████║
 ╚══════╝╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝
```

[![CI](https://github.com/MenkeTechnologies/zshrs/actions/workflows/ci.yml/badge.svg)](https://github.com/MenkeTechnologies/zshrs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/zshrs.svg)](https://crates.io/crates/zshrs)
[![Downloads](https://img.shields.io/crates/d/zshrs.svg)](https://crates.io/crates/zshrs)
[![Docs.rs](https://docs.rs/zshrs/badge.svg)](https://docs.rs/zshrs)
[![Docs](https://img.shields.io/badge/docs-online-blue.svg)](https://menketechnologies.github.io/zshrs/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

### `[THE FIRST COMPILED UNIX SHELL]`

> *"No fork, no problems."*

The first Unix shell to compile to bytecodes and execute on a purpose-built virtual machine with fused superinstructions. Since the Bourne shell at Bell Labs in 1970, every Unix shell has been an interpreter. zshrs is the first to be a compiler. A drop-in zsh replacement written in Rust — 190k+ lines, 267 source files, 80 core modules, 26 ZLE widgets, 48 fish-ported builtins, persistent worker pool, AOP intercept, SQLite FTS5 caching, and full zsh compatibility.

### [`Docs`](https://menketechnologies.github.io/zshrs/zshrs.html) · [`Coverage Report`](https://menketechnologies.github.io/zshrs/report.html) · [`strykelang`](https://github.com/MenkeTechnologies/strykelang) · [`fusevm`](https://github.com/MenkeTechnologies/fusevm) · [`compsys`](compsys/)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Install](#0x01-install)
- [\[0x02\] No-Fork Architecture](#0x02-no-fork-architecture)
- [\[0x03\] Bytecode Compilation](#0x03-bytecode-compilation)
- [\[0x04\] Concurrent Primitives](#0x04-concurrent-primitives)
- [\[0x05\] AOP Intercept](#0x05-aop-intercept)
- [\[0x06\] Worker Thread Pool](#0x06-worker-thread-pool)
- [\[0x07\] SQLite Caching](#0x07-sqlite-caching)
- [\[0x08\] Exclusive Builtins](#0x08-exclusive-builtins)
- [\[0x09\] Compatibility](#0x09-compatibility)
- [\[0x0A\] Architecture](#0x0a-architecture)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

zshrs replaces `fork + exec` with a persistent worker thread pool, compiles every command to [fusevm](https://github.com/MenkeTechnologies/fusevm) bytecodes, caches compiled chunks in SQLite, and runs the completion system on FTS5 indexes. The result: shell startup, command dispatch, globbing, completion, and autoloading are all faster by orders of magnitude.

---

## [0x01] INSTALL

```sh
# From crates.io
cargo install zshrs

# From source — lean build, pure shell, no stryke dependency
git clone https://github.com/MenkeTechnologies/zshrs
cd zshrs && cargo build --release
# binary: target/release/zshrs

# Set as login shell
sudo sh -c 'echo ~/.cargo/bin/zshrs >> /etc/shells'
chsh -s ~/.cargo/bin/zshrs
```

---

## [0x02] NO-FORK ARCHITECTURE

Every operation that zsh forks for runs in-process. **Zero forks for builtins.**

| Operation | zsh | zshrs |
|-----------|-----|-------|
| `$(cmd)` | fork + pipe | In-process stdout capture via `dup2` |
| `<(cmd)` / `>(cmd)` | fork + FIFO | Worker pool thread + FIFO |
| `cat file` | fork + exec /bin/cat | **Builtin** — zero fork |
| `head`/`tail`/`wc` | fork + exec | **Builtin** — zero fork |
| `sort`/`find`/`uniq` | fork + exec | **Builtin** — zero fork |
| `date`/`hostname`/`uname` | fork + exec | **Builtin** — direct syscall |
| `sleep`/`mktemp`/`touch` | fork + exec | **Builtin** — zero fork |
| `xattr` operations | fork + exec xattr | **Direct syscall** — zero fork |
| `pmap`/`pgrep`/`peach` | fork N times | **VM execution** — zero fork |
| `**/*.rs` | Single-threaded `opendir` | Parallel `walkdir` per-subdir on pool |
| `*(.x)` qualifiers | N serial `stat` calls | One parallel metadata prefetch |
| `rehash` | Serial `readdir` per PATH dir | Parallel scan across pool |
| `compinit` | Synchronous fpath scan | Background scan + bytecode compilation |
| History write | Synchronous `fsync` | Fire-and-forget to pool |
| Autoload | Read file + parse every time | Bytecode deserialization from SQLite |
| Plugin source | Parse + execute every startup | Delta replay from SQLite cache |

### Coreutils Builtins (Anti-Fork)

23 coreutils commands run in-process with zero fork overhead:

```
cat  head  tail  wc  sort  find  uniq  cut  tr  seq  rev  tee
basename  dirname  touch  realpath  sleep  whoami  id  hostname
uname  date  mktemp
```

**Speedup: 2000-5000x** per invocation (2-5ms fork overhead → 0.001ms builtin call).

---

## [0x03] BYTECODE COMPILATION

Every command compiles to [fusevm](https://github.com/MenkeTechnologies/fusevm) bytecodes:

```
Interactive command  ──► Parser ──► ShellCompiler ──► fusevm::Op ──► VM::run()
Script file (first)  ──► Parser ──► ShellCompiler ──► VM::run() ──► cache in SQLite
Script file (cached) ──► SQLite ──► deserialize Chunk ──► VM::run()
                         (no lex, no parse, no compile)
Autoload function    ──► SQLite ──► deserialize Chunk ──► VM::run()
                         (microseconds)
```

The shell compiler targets the same `Op` enum that [strykelang](https://github.com/MenkeTechnologies/strykelang) uses. Both frontends share fused superinstructions, extension dispatch, and the Cranelift JIT path.

### Execution Pipeline

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Script file                                                            │
│       │                                                                 │
│       ▼                                                                 │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │ SQLite bytecode cache                                           │   │
│  │   check_bytecode(path, mtime) → Option<Vec<u8>>                 │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│       │                                                                 │
│       ├─── HIT (100x faster) ────────────────────────┐                 │
│       │                                               │                 │
│       ▼ MISS                                          ▼                 │
│  Lexer → Parser → ShellCompiler ────────────► fusevm::Chunk            │
│                         │                             │                 │
│                         ▼                             │                 │
│                  store_bytecode()                     │                 │
│                                                       │                 │
│       ┌───────────────────────────────────────────────┘                │
│       ▼                                                                 │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                    fusevm::VM::run()                            │   │
│  │                                                                 │   │
│  │  ┌───────────────────────────────────────────────────────────┐ │   │
│  │  │ JIT eligibility check                                     │ │   │
│  │  └───────────────────────────────────────────────────────────┘ │   │
│  │       │                                                         │   │
│  │       ├─── Block JIT (loops, branches) ──► Cranelift ──► x86-64│   │
│  │       │                                                         │   │
│  │       ├─── Linear JIT (straight-line) ──► Cranelift ──► x86-64 │   │
│  │       │                                                         │   │
│  │       ▼ Fallback                                                │   │
│  │  ┌───────────────────────────────────────────────────────────┐ │   │
│  │  │ Interpreter: jump table dispatch + fused superinstructions│ │   │
│  │  └───────────────────────────────────────────────────────────┘ │   │
│  └─────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
```

| Tier | What | When |
|------|------|------|
| **SQLite cache** | Skip lex/parse/compile | Warm script runs |
| **Block JIT** | Native x86-64 via Cranelift | Loops, conditionals |
| **Linear JIT** | Native x86-64 via Cranelift | Straight-line arithmetic |
| **Interpreter** | Jump table + superinstructions | Builtins, I/O, strings |

**Benchmark: 100x warm start speedup**

```
Cold (cache miss):  717ms  — lex + parse + compile + cache write + execute
Warm (cache hit):     7ms  — deserialize + execute
```

---

## [0x04] CONCURRENT PRIMITIVES

Full parallelism in the lean binary. No stryke dependency needed.

```zsh
# Async/await
id=$(async 'sleep 5; curl https://api.example.com')
result=$(await $id)

# Parallel map — ordered output
pmap 'gzip {}' *.log

# Parallel filter
pgrep 'grep -q TODO {}' **/*.rs

# Parallel for-each — unordered, fire as completed
peach 'convert {} {}.png' *.svg

# Barrier — run all, wait for all
barrier 'npm test' ::: 'cargo test' ::: 'pytest'
```

---

## [0x05] AOP INTERCEPT

First shell with aspect-oriented programming:

```zsh
# Before — log every git command
intercept before git { echo "[$(date)] git $INTERCEPT_ARGS" >> ~/git.log }

# After — timing
intercept after '_*' { echo "$INTERCEPT_NAME took ${INTERCEPT_MS}ms" }

# Around — memoize
intercept around expensive_func {
    local cache=/tmp/cache_${INTERCEPT_ARGS// /_}
    if [[ -f $cache ]]; then cat $cache
    else intercept_proceed | tee $cache; fi
}
```

---

## [0x06] WORKER THREAD POOL

Persistent pool of [2-18] threads. Configurable:

```toml
# ~/.config/zshrs/config.toml
[worker_pool]
size = 8

[completion]
bytecode_cache = true

[history]
async_writes = true

[glob]
parallel_threshold = 32
recursive_parallel = true
```

---

## [0x07] SQLITE CACHING

Three databases power the shell:

| Database | Purpose |
|----------|---------|
| **compsys.db** | Completions: autoloads with bytecodes, comps, services, PATH executables (FTS5) |
| **history.db** | Frequency-ranked, timestamped, duration, exit status per command |
| **plugins.db** | Plugin delta cache: functions, aliases, variables, hooks, zstyles, options |

Browse without SQL:

```zsh
dbview                        # list tables + row counts
dbview autoloads _git         # single function: source, body, bytecode status
dbview comps git              # search completions
dbview history docker         # search history
```

---

## [0x08] EXCLUSIVE BUILTINS

### Parallel Primitives (VM-executed, zero fork)

| Builtin | Description |
|---------|-------------|
| `async` / `await` | Ship work to pool, collect result |
| `pmap` | Parallel map with ordered output — runs on VM, not fork |
| `pgrep` | Parallel filter — runs on VM, not fork |
| `peach` | Parallel for-each, unordered — runs on VM, not fork |
| `barrier` | Run all commands in parallel, wait for all |

### AOP / Debugging

| Builtin | Description |
|---------|-------------|
| `intercept` | AOP before/after/around advice on any command |
| `intercept_proceed` | Call original from around advice |
| `doctor` | Full diagnostic: pool metrics, cache stats, bytecode coverage |
| `dbview` | Browse SQLite caches without SQL |
| `profile` | In-process command profiling with nanosecond accuracy |

### Coreutils (Anti-Fork)

| Builtin | Description |
|---------|-------------|
| `cat` | Concatenate files — no fork |
| `head` / `tail` | First/last N lines — no fork |
| `wc` | Line/word/char count — no fork |
| `sort` / `uniq` | Sort and dedupe — no fork |
| `find` | Walk directories — no fork |
| `cut` / `tr` / `rev` | Text manipulation — no fork |
| `seq` | Number sequences — no fork |
| `tee` | Copy stdin to files — no fork |
| `date` | Current date/time — direct syscall |
| `sleep` | Delay — no fork |
| `mktemp` | Create temp file/dir — no fork |
| `hostname` / `uname` / `id` / `whoami` | System info — direct syscall |
| `touch` / `realpath` / `basename` / `dirname` | File ops — no fork |
| `zgetattr` / `zsetattr` / `zdelattr` / `zlistattr` | xattr ops — direct syscall |

---

## [0x09] COMPATIBILITY

- Full zsh script compatibility — runs existing `.zshrc`
- Full bash compatibility via emulation
- Fish-style syntax highlighting, autosuggestions, abbreviations
- **180+ builtins** (150 zsh + 23 coreutils + parallel primitives)
- ZWC precompiled function support
- Glob qualifiers, parameter expansion flags, completion system
- zstyle, ZLE widgets, hooks, modules
- `--posix` mode for strict POSIX compliance

---

## [0x0A] ARCHITECTURE

```
                  ┌──────────────────────────────────────────┐
                  │              zshrs binary                 │
                  ├──────────────┬───────────────────────────┤
                  │   src/ (80)  │      fish/ (48 builtins)  │
                  │   lexer      │      reader + line editor  │
                  │   parser     │      syntax highlighting   │
                  │   compiler   │      autosuggestions        │
                  │   exec       │      abbreviations          │
                  │   jobs       │      env dispatch           │
                  │   signals    │      history backend        │
                  │   params     │      process control        │
                  │   glob       │      event system           │
                  │   zle/ (26)  │                             │
                  ├──────────────┴───────────────────────────┤
                  │             compsys (27 files)            │
                  │   SQLite FTS5 · menuselect · zstyle      │
                  ├────────���─────────────────────────────────┤
                  │           fusevm (bytecode VM)            │
                  │   129 opcodes · fused loops · JIT path   │
                  └──────────────────────────────────────────┘
```

---

## [0xFF] LICENSE

MIT — Copyright (c) 2026 [MenkeTechnologies](https://github.com/MenkeTechnologies)
