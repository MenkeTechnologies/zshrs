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

## `[PATENT PENDING]`

The first Unix shell to compile to bytecodes and execute on a purpose-built virtual machine with fused superinstructions. Since the Bourne shell at Bell Labs in 1970, every Unix shell has been an interpreter. zshrs is the first to be a compiler. A drop-in zsh replacement written in Rust — **568k+ lines, 598 source files** across a 2-crate workspace (`zshrs` runtime + `zshrs-daemon`; `compsys` was folded into the runtime), with the runtime split into a **strict 1:1 port directory** (`src/ported/` — 107 files, every fn maps to a real `Src/<x>.c` zsh function, enforced by `tests/port_purity.rs`), **a non-port extensions directory** (`src/extensions/` — 40 files, features zsh C does not have: AOT, daemon coordination, plugin/script/autoload caches, fish-style autosuggest/abbrev/highlight, persistent worker pools, ZWC byte-code helpers), and a feature-gated recorder (`src/recorder/`). **100% ZLE widget coverage** (193/193 entries from zsh's `Src/Zle/iwidgets.list` — history navigation, vi find/repeat/marks, undo/redo, isearch, yank-pop, shell-aware word motion, region/visual mode, text objects, completion menu, $zle_highlight parsing), 64 fish-ported builtins, persistent worker pool, AOP intercept, **rkyv**-backed bytecode images (mmap hot path; the only shell bytecode cache), **read-only SQLite mirrors** beside them for `dbview` / SQL inspection only (no cache semantics), and full zsh compatibility.

### [`Read the Docs`](https://menketechnologies.github.io/zshrs/index.html) &middot; [`Reference`](https://menketechnologies.github.io/zshrs/reference.html) · [`Coverage Report`](https://menketechnologies.github.io/zshrs/report.html) · [`Compsys Port Report`](docs/compsys_port_report.html) · [`strykelang`](https://github.com/MenkeTechnologies/strykelang) · [`fusevm`](https://github.com/MenkeTechnologies/fusevm) · [`compsys`](compsys/)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Install](#0x01-install)
- [\[0x02\] No-Fork Architecture](#0x02-no-fork-architecture)
- [\[0x03\] Bytecode Compilation](#0x03-bytecode-compilation)
- [\[0x04\] Concurrent Primitives](#0x04-concurrent-primitives)
- [\[0x05\] AOP Intercept](#0x05-aop-intercept)
- [\[0x06\] Worker Thread Pool](#0x06-worker-thread-pool)
- [\[0x07\] RKYV cache layout](#0x07-rkyv-cache-layout)
- [\[0x08\] Exclusive Builtins](#0x08-exclusive-builtins)
- [\[0x09\] Shell Language Features](#0x09-shell-language-features)
- [\[0x0A\] Compatibility](#0x0a-compatibility)
- [\[0x0B\] Architecture](#0x0b-architecture)
- [\[0x0C\] Editor Integration](#0x0c-editor-integration)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

zshrs replaces `fork + exec` with a persistent worker thread pool, compiles every command to [fusevm](https://github.com/MenkeTechnologies/fusevm) bytecodes, and **persists compiled chunks only in rkyv shards** under `~/.zshrs/images/` (single-directory rule — every zshrs file lives under `$ZSHRS_HOME` / `~/.zshrs/`; see [`docs/DAEMON.md`](docs/DAEMON.md)). Beside that tree, **`catalog.db` and related SQL views are read-only mirrors** for inspection (`dbview`, ad-hoc SQL): daemon-hydrated, **never authoritative for cache hit/miss or execution**. They are not a second shell cache. **`history.db`** holds history only — it is unrelated to bytecode caching. The result: shell startup, command dispatch, globbing, completion, and autoloading are all faster by orders of magnitude.

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
| Autoload | Read file + parse every time | Bytecode mmap + zero-copy load from **rkyv** |
| Plugin source | Parse + execute every startup | Delta replay from **rkyv** image |

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

Every command compiles to [fusevm](https://github.com/MenkeTechnologies/fusevm) bytecodes via a faithful port of zsh's lexer + parser:

```
Interactive command  ──► lex::zshlex ──► parse::parse ──► ZshCompiler ──► fusevm::Op ──► VM::run()
                         (port of    (port of      (original;
                          Src/lex.c)  Src/parse.c)  ~1.4k LOC)
Script file (first)  ──► lex::zshlex ──► parse::parse ──► ZshCompiler ──► VM::run() ──► persist rkyv shard
Script file (cached) ──► index.rkyv + mmap shard ──► deserialize Chunk ──► VM::run()
                         (no lex, no parse, no compile)
Autoload function    ──► rkyv shard ──► deserialize Chunk ──► VM::run()
                         (microseconds)
```

The lexer and parser are direct ports from zsh's C source (`Src/lex.c`, `Src/parse.c`); only the bytecode compiler is original Rust. The 4-tier `ZshProgram → ZshList → ZshSublist → ZshPipe → ZshCommand` AST is preserved verbatim from zsh, ensuring per-construct behavior parity. The bytecode compiler targets the same `Op` enum that [strykelang](https://github.com/MenkeTechnologies/strykelang) uses. Both frontends share fused superinstructions, extension dispatch, and the Cranelift JIT path.

### Execution Pipeline

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Script file                                                            │
│       │                                                                 │
│       ▼                                                                 │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │ rkyv bytecode cache (images/*.rkyv + index.rkyv)                 │   │
│  │   lookup(path, mtime) → mmap'd fusevm::Chunk                     │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│       │                                                                 │
│       ├─── HIT (100x faster) ────────────────────────┐                 │
│       │                                               │                 │
│       ▼ MISS                                          ▼                 │
│  lex+parse → ZshCompiler ────────► fusevm::Chunk            │
│                         │                             │                 │
│                         ▼                             │                 │
│                  persist_shard()                      │                 │
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
| **rkyv image hit** | Skip lex/parse/compile | Warm script runs |
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
# ~/.zshrs/zshrs.toml  (single-directory rule; configurable via $ZSHRS_HOME)
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

## [0x07] RKYV CACHE LAYOUT

Compiled bytecode and plugin/autoload payloads live in **rkyv** under `~/.zshrs/images/`:

| Path | Purpose |
|------|---------|
| **`index.rkyv`** | Top-level index: fq_name → shard id, generation, byte offset |
| **`images/{hash8}-*.rkyv`** | Mmap-ready shards (system, completions, plugins, scripts, `.zshrc`, …) |

**SQLite (read-only mirrors)** — same directory, different job: daemon-maintained copies you can query with SQL or `dbview`. They are **not** the bytecode cache and are **not** read when deciding cache hit/miss or when running compiled code.

| Store | Purpose |
|-------|---------|
| **`catalog.db`** | Joinable mirror of catalog metadata (human / tooling reads only) |
| **`history.db`** | Command history persistence (orthogonal to bytecode caching — not a cache layer for compiled chunks) |
| **Mirror / FTS views** | Optional SQL-side views of names and paths for `dbview` — read-only; see [`docs/DAEMON.md`](docs/DAEMON.md) |

Browse mirrors without SQL:

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
| `dbview` | Read-only browse of SQLite **mirrors** (not the rkyv cache) |
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

## [0x09] SHELL LANGUAGE FEATURES

Every shell construct compiles to fusevm bytecode — no tree-walker dispatch lives in zshrs. The [full reference](https://menketechnologies.github.io/zshrs/reference.html) documents each entry with a runnable example.

### Control flow

```zsh
# Standard POSIX/zsh control structures — all compile to fusevm bytecode
if [[ -d $dir ]]; then …; elif [[ -f $dir ]]; then …; else …; fi
while (( i < 10 )); do …; done
until ping -c1 host >/dev/null; do sleep 1; done
for f in *.rs; do echo "$f"; done
for ((i=0; i<10; i++)); do …; done
case $cmd in start) … ;; stop) … ;; *) … ;; esac
select choice in build test deploy; do … done   # interactive numbered menu
coproc { while read l; do echo "ECHO: $l"; done } # bidirectional pipe
```

### Indexed arrays

```zsh
arr=(alpha beta gamma)            # literal
arr+=(delta epsilon)              # append
echo ${arr[1]}                    # alpha (1-based)
echo ${arr[-1]}                   # epsilon (negative from end)
echo ${arr[@]}                    # splice — N argv slots
echo ${#arr[@]}                   # length
for x in ${arr[@]}; do …; done    # iterate (flattens via BUILTIN_ARRAY_FLATTEN)
```

### Associative arrays

```zsh
typeset -A m                      # declare
m[name]=Jacob; m[role]=eng        # set
echo "${m[name]}"                 # lookup
for k in "${(k)m}"; do echo $k; done   # keys
for v in "${(v)m}"; do echo $v; done   # values
```

### Parameter expansion flags (zsh-style)

```zsh
echo ${(L)var}                    # lowercase
echo ${(U)var}                    # uppercase
echo ${(j: :)arr}                 # join with space
echo ${(s:,:)scalar}              # split on comma → array
echo ${(f)$(cmd)}                 # split on newlines
echo ${(o)arr}                    # sort ascending
echo ${(O)arr}                    # sort descending
echo ${(P)ref}                    # indirect lookup
echo ${(jL)arr}                   # stack: join then lowercase
echo ${(s:,:U)scalar}             # stack: split then uppercase
```

### Parameter expansion forms

```zsh
${var:-default}                   # default if unset/empty
${var:=default}                   # assign default
${var:?msg}                       # error if unset
${var:+alt}                       # alternate if set
${#var}                           # length
${var:offset:length}              # substring
${var#pat} / ${var##pat}          # strip shortest/longest prefix
${var%pat} / ${var%%pat}          # strip shortest/longest suffix
${var/pat/repl} / ${var//pat/repl} # replace first/all
${var:u} / ${var:l}               # upper/lower case (zsh postfix)
```

### Background, async, coprocesses

```zsh
sleep 30 &                        # fork + setsid; parent gets Status(0)
jobs; fg %1; wait $!              # job control
async 'expensive-task' | xargs await   # worker-pool, no fork
coproc { body }                   # bidirectional pipe; $COPROC=[rd_fd, wr_fd]
echo hi >/dev/fd/${COPROC[2]}     # write to coproc stdin
read line </dev/fd/${COPROC[1]}   # read from coproc stdout
```

### Eval, dynamic dispatch, AOP

```zsh
eval 'echo $x'                    # single-quoted args defer expansion correctly
cmd=ls; $cmd -la                  # dynamic command name routes through host intercepts
intercept before git { …; }       # AOP advice fires for both literal and dynamic invocations
```

---

## [0x0A] COMPATIBILITY

- Full zsh script compatibility — runs existing `.zshrc`
- Full bash compatibility via emulation
- Fish-style syntax highlighting, autosuggestions, abbreviations
- **180+ builtins** (150 zsh + 23 coreutils + parallel primitives) — see the [Reference](https://menketechnologies.github.io/zshrs/reference.html) for the full catalog
- ZWC precompiled function support
- Glob qualifiers, parameter expansion flags, completion system
- zstyle, ZLE widgets, hooks, modules
- `--posix` mode for strict POSIX compliance

### Test corpus parity

| Suite | Tests | Coverage |
|-------|-------|----------|
| `zsh_construct_corpus` | 392 | Every sh/zsh construct outside modules |
| `zsh_corpus_via_new_pipeline` | 123 | Native lex+parse+ZshCompiler path |
| `no_tree_walker_dispatch` | 158 | Behavioral pins for the no-tree-walker invariant |
| `compile_zsh_smoke` | 28 | Per-construct bytecode-level smoke |
| `tree_walker_absent` | 8 | Source-level absence checks (anti-regression) |
| `zsh_parser_probe` | 91 | AST-shape probes for every construct |
| `ztst_runner` | 70 | Real `.ztst` files from upstream zsh |
| **Total** | **876** | All green on the new (default) pipeline |

---

## [0x0B] ARCHITECTURE

The codebase is **structurally divided into ported code vs extensions**, with the boundary mechanically enforced by `tests/port_purity.rs`. Bots, contributors, and humans all read [`docs/PORT.md`](docs/PORT.md) before writing a single line.

```
                  ┌────────────────────────────────────────────────────────────────┐
                  │                       zshrs workspace                          │
                  │             2 crates · 598 .rs files · 568k+ lines             │
                  ├──────────────────────────────────────────┬─────────────────────┤
                  │  src/ (285 .rs — runtime crate)           │  fish/ (157 .rs)    │
                  │  ┌────────────────────────────────────┐   │  reader / line edit │
                  │  │  src/ported/  (107 — STRICT PORT)  │   │  syntax highlight   │
                  │  │  every .rs ↔ a real Src/<x>.c file │   │  autosuggest        │
                  │  │  every fn carries `/// Port of …`  │   │  abbreviations      │
                  │  │  enforced by tests/port_purity.rs  │   │  env dispatch       │
                  │  │  builtins/ · zle/ · modules/ ·     │   │  history backend    │
                  │  │  hist · jobs · params · pattern ·  │   │  process control    │
                  │  │  signals · glob · subst · math ·   │   │  event system       │
                  │  │  prompt · utils · init · …         │   ├─────────────────────┤
                  │  └────────────────────────────────────┘   │  parse + lex now    │
                  │  ┌────────────────────────────────────┐   │  live IN-RUNTIME    │
                  │  │  src/extensions/  (40 — NON-PORT)  │   │  (folded from the   │
                  │  │  features zsh C does NOT have:     │   │  old parse crate)   │
                  │  │  AOT · plugin/script/autoload      │   ├─────────────────────┤
                  │  │  cache · fish_features · worker    │   │  daemon/ (41 .rs)   │
                  │  │  pool · zwc · arith_compiler ·     │   │  zshrs-daemon — IPC │
                  │  │  hooks · keymaps · widgets ·       │   │  · HTTP · OpenAPI · │
                  │  │  daemon_presence · log · …         │   │  fsnotify · cache · │
                  │  └────────────────────────────────────┘   │  zsource/zhistory/  │
                  │  ┌────────────────────────────────────┐   │  zjob builtins      │
                  │  │  src/recorder/  (1 — feature gate) │   ├─────────────────────┤
                  │  │  AOP intercept; #[cfg(recorder)]   │   │ compsys folded into │
                  │  │  → zero bytes in default binary    │   │ runtime (was its    │
                  │  └────────────────────────────────────┘   │ own crate)          │
                  ├──────────────────────────────────────────┴─────────────────────┤
                  │                  bins/  (4 — entry points)                     │
                  │     zshrs            zshrs-recorder         zd                 │
                  │     (default)        (--features recorder)  (--features zd)    │
                  ├────────────────────────────────────────────────────────────────┤
                  │                       fusevm (bytecode VM)                    │
                  │            129 opcodes · fused superinstructions · JIT         │
                  └────────────────────────────────────────────────────────────────┘
```

### Directory rule (PORT.md)

| Directory | Rule | Enforcement |
|-----------|------|-------------|
| `src/ported/` | **Strict 1:1 port.** Every `.rs` mirrors a real `src/zsh/Src/<x>.c`; every top-level `fn` carries `/// Port of <cname>() from Src/<file>.c:NNNN`; no invented helpers; **directory and file set FROZEN** (107 files, no new files allowed). | `tests/port_purity.rs` |
| `compsys/ported/` | **1:1 mirror of zsh's `Completion/` tree.** Engine functions (`Base/{Completer,Core,Utility,Widget}`, `Zsh/Context`, plus engine-only entries in `Unix/Type`, `Zsh/Type`, `Zsh/Command`, `Unix/Command`, and top-level `compinit`/`compdump`) are ported to Rust as `<name>.rs` and carry a `Port of _<NAME>` header citing the upstream shell source. End-user shell completers (`*/Command`, `Zsh/Function`, end-user type files) are **copied as-is alongside** the Rust ports — same dir layout, same filename, no `.rs` extension — and dispatched via the `_call_function` bridge. Current coverage: **992 upstream files mirrored, 125 engine .rs ports** (3 engine scripts still shell-only). Regenerate the coverage report with `scripts/gen_compsys_port_report.py` → `docs/compsys_port_report.html`. | per-fn tests; doc-comment shell-source citations; `gen_compsys_port_report.py` |
| `src/extensions/` | **Non-port only.** Features zsh C demonstrably does *not* have. Must not duplicate or shadow any port. | `port_purity` exempts the 1:1 file rule for this directory only |
| `src/recorder/` | **Feature-gated.** Every symbol `#[cfg(feature = "recorder")]`; deleted by rustc when off. | `Cargo.toml` `required-features = ["recorder"]` on the `zshrs-recorder` bin |
| `src/zsh/` | **Read-only reference.** Vendored upstream zsh C source. The spec; never modified. | n/a |

---

## [0x0C] EDITOR INTEGRATION

zshrs ships an **LSP server** and **DAP debug adapter** built into the
binary, plus a **JetBrains IDE plugin** that drives both.

### CLI flags

```sh
zshrs --lsp                  # LSP server over stdio
zshrs --dap HOST:PORT        # DAP debugger; connect-back to IDE listener
zshrs --dump-reflection      # JSON dump of builtins / keywords / options
zshrs --docs <name>          # render the LSP hover card for <name>
```

All four flags dispatch from `bins/zshrs.rs` into `src/extensions/lsp.rs`
and `src/extensions/dap.rs`. Both modules are dependency-free additions
(no `lsp-server` / `lsp-types` / `dap-types` crates) — Content-Length
framing + JSON-RPC are hand-rolled on top of `serde_json` to keep the
default build lean.

### LSP capabilities (`zshrs --lsp`)

| Capability                          | Trigger                                  |
|-------------------------------------|------------------------------------------|
| `completion`                        | builtins, keywords, options, special vars, in-file functions |
| `hover`                             | markdown cards for builtins / keywords / options / special vars |
| `definition` / `references`         | function names declared in the open document |
| `documentHighlight`                 | same scan as references                  |
| `documentSymbol`                    | `function foo`, `foo()`, `alias`, `local`/`typeset`/`export` |
| `foldingRange`                      | `{ … }` / `do … done` / `case … esac` blocks + ≥3 `#` comment runs |
| `rename` (with `prepareRename`)     | word-boundary aware replace across document |
| `semanticTokens/full`               | comment / string / number / keyword / variable / function classes |
| `formatting`                        | trailing-whitespace strip, leading-indent normalize, final newline |
| `publishDiagnostics`                | brace + block matching, unclosed strings, lights up on `didOpen` / `didChange` / `didSave` |

Trigger characters for completion: `$`, `{`, `-`, `:`. Optional
`ZSHRS_LSP_LOG=<path>` env var dumps every request/response for debugging.

### DAP capabilities (`zshrs --dap HOST:PORT`)

| Request                               | Behaviour (v1)                          |
|---------------------------------------|-----------------------------------------|
| `initialize` / `configurationDone`    | full capability advertisement, emits `initialized` event |
| `setBreakpoints`                      | stored per-file, ack with `verified: true` |
| `launch`                              | spawn `zshrs <program> <args>` as a child process |
| `threads` / `stackTrace` / `scopes`   | single-thread model, one synthetic frame |
| `variables`                           | environment snapshot (scope ref 1)      |
| `evaluate`                            | runs `zshrs -c <expr>` against current `cwd`, returns stdout |
| `continue` / `next` / `stepIn` / `stepOut` | acked (no per-statement pause in v1) |
| `pause`                               | emits `stopped { reason: "pause" }`     |
| `disconnect` / `terminate`            | kills the child process                 |
| program stdout / stderr               | streamed as DAP `output` events         |
| child exit                            | fires `terminated` event                |

Deeper integration (per-statement pause, breakpoint honouring against the
live interpreter, scope walk-back into the `Param` table) is scaffolded
via `dap::install_hooks(DapHooks { … })` and lands incrementally.

### JetBrains plugin (`editors/intellij/`)

```sh
cd editors/intellij
JAVA_HOME=$(/usr/libexec/java_home -v 17) ./gradlew buildPlugin
# → build/distributions/zshrs-intellij-<version>.zip
```

Install via *Settings → Plugins → ⚙ → Install Plugin from Disk…*.

Plugin features:

- **File types**: `.zsh` + every dot-rc (`.zshrc`, `.zshenv`, `.zlogin`,
  `.zlogout`, `.zprofile`, `.zpreztorc`).
- **Hand-rolled lexer** with 42 independently-themeable color slots
  (*Settings → Editor → Color Scheme → zshrs*).
- **LSP client** auto-starts `zshrs --lsp` on first file open. Hover,
  completion, goto-definition, references, rename, document symbols,
  semantic tokens, folding, formatting, diagnostics — all wired.
- **Run configurations** with toggles for `-f`/`-x`/`-v`/`--disasm`/
  `--dump-ast` and a compat-mode picker (`zsh` / `bash` / `ksh` /
  `posix` / default `zshrs`).
- **Debugger** over DAP TCP socket: line breakpoints from the gutter,
  step over/into/out/pause, frames panel with source navigation,
  variables panel (scalars + arrays + assoc arrays), Evaluate dialog,
  Console streaming program stdout in real time.
- **Reflection tool window** (right edge) — left-click any name to open
  the ANSI-rendered `zshrs --docs <name>` card.
- **Settings → Tools → zshrs** — point at a non-PATH `zshrs` binary, set
  LSP extra args + env, configure file extensions, control auto-restart.

Requires a paid JetBrains IDE on 2024.2+ (RustRover, IDEA Ultimate,
GoLand, PyCharm Pro, WebStorm, RubyMine, PhpStorm, CLion, Rider,
DataGrip, Aqua) because the platform LSP API is not in Community
editions.

### Other LSP / DAP clients

The stdio LSP and TCP DAP servers are protocol-conformant — any
LSP/DAP client works:

```toml
# Helix — ~/.config/helix/languages.toml
[language-server.zshrs]
command = "zshrs"
args = ["--lsp"]

[[language]]
name = "bash"
language-servers = ["zshrs"]
file-types = ["zsh", "zshrc", "zshenv", "zlogin", "zlogout", "zprofile"]
```

```lua
-- Neovim (nvim-lspconfig style)
require("lspconfig").configs.zshrs = {
  default_config = {
    cmd = { "zshrs", "--lsp" },
    filetypes = { "zsh", "sh" },
    root_dir = function() return vim.fn.getcwd() end,
  },
}
require("lspconfig").zshrs.setup({})
```

```jsonc
// VS Code — keybindings.json + a small extension that spawns `zshrs --lsp`
// over stdio works the same as any LSP-backed extension.
```

See [`editors/intellij/README.md`](editors/intellij/README.md) for the
JetBrains plugin's full architecture, debugger internals, and limitation
list.

---

## [0xFF] LICENSE

MIT — Copyright (c) 2026 [MenkeTechnologies](https://github.com/MenkeTechnologies)

Original-authorship record + portability stance:
[CREATORS.md](CREATORS.md). Maintainer governance + protected
invariants: [MAINTAINERS.md](MAINTAINERS.md).

**This is a legacy, not a battle.** The synthesis
(compiled-shell architecture, 90/10 daemon split,
recorder-owns-rebuild AOP intercept, single `~/.zshrs/` rule,
session-persistent supervised jobs with bidirectional ptmx
attach, cross-shell pub/sub + named-lock builtins, auto-derived
OpenAPI surface, flat-text history + sibling FTS5 index) is
prior art for the shell-design commons under the MIT grant.
Future shells — bash, fish, nushell, elvish, oil, xonsh, murex,
projects that don't exist yet — should inherit any of it. The
protected invariants in `MAINTAINERS.md` guard upstream
identity, not the ideas.

**Ports must credit zshrs as the invention source in their
docs** — a one-line attribution in your README / design doc /
release notes. Suggested wording:

> Inspired by / ported from
> [zshrs](https://github.com/MenkeTechnologies/zshrs) by
> Jacob Menke (MenkeTechnologies).

Ideas can't be copyrighted so this is an ask, not an
MIT-enforced clause; honoring it keeps the legacy traceable.
See [CREATORS.md § Legacy](CREATORS.md#legacy) +
[§ Attribution expectation](CREATORS.md#attribution-expectation)
for the full list + suggested forms.
