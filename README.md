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

### `[THE FIRST JIT-COMPILED UNIX SHELL]`

> *"No fork, no problems."*

The first Unix shell to JIT-compile to native machine code. Bytecode alone is no longer a first: Nushell's IR compiler and evaluator landed in 0.96.0 and became its default evaluator in 0.98.0 — but that IR is interpreted, compiled per parse, and discarded at process exit. zshrs compiles every command (interactive, script, function, sourced file) to fusevm bytecode with fused superinstructions, hands hot blocks to a tiered Cranelift JIT that emits x86-64/aarch64 machine code, and persists the bytecode across processes in rkyv images. No shell before it ran shell source as native code. A drop-in zsh replacement written in Rust — **880k lines, 832 source files** across a 3-crate workspace (`zshrs` runtime + `zshrs-daemon` + `znative`, the published plugin-ABI SDK; `compsys` was folded into the runtime), with the runtime split into a **strict 1:1 port directory** (`src/ported/` — 106 files, every fn maps to a real `Src/<x>.c` zsh function, enforced by `tests/port_purity.rs`), **a non-port extensions directory** (`src/extensions/` — 94 files, features zsh C does not have: AOT, daemon coordination, plugin/script/autoload caches, native fish-ported ZLE engines (syntax highlight, autosuggest, history search, autopair — opt-in via `[zle]` in `~/.zshrs/zshrs.toml`, so bare `zshrs -f` stays byte-identical to `zsh -f`), persistent worker pools, ZWC byte-code helpers), and a feature-gated recorder (`src/recorder/`). **193 ZLE widgets registered** in `IWIDGET_NAMES` (history navigation, vi find/repeat/marks, undo/redo, isearch, yank-pop, shell-aware word motion, region/visual mode, text objects, completion menu, $zle_highlight parsing), 47 fish-ported builtins, persistent worker pool, AOP intercept, **rkyv**-backed bytecode images (mmap hot path; the only persisted shell bytecode cache — zsh's `.zwc` is wordcode for zsh's own interpreter, Nushell's IR never leaves the process), **read-only SQLite mirrors** beside them for `dbview` / SQL inspection only (no cache semantics), and full zsh compatibility. Also **the first shell to expose its native-plugin interface as a stable, versioned, independently-published ABI** — third parties `cargo add znative`, ship a `cdylib`, and load it at runtime via `zmodload -R` (version-gated, mismatches refused). bash `enable -f` and zsh `zmodload` load native code too, but only compiled against the shell's private internal headers, welded to one build with no stable ABI; see [`docs/PLUGINS.md`](docs/PLUGINS.md). And through its companion build [`zshrs-native`](https://github.com/MenkeTechnologies/zshrs-native), **the first shell with a version control system compiled into it** — `git` served natively as a builtin in the shell's own process, no fork, no exec, no PATH lookup. Since the Bourne shell in 1970 every Unix shell has run git as a foreign binary; BusyBox has no git applet, Nushell's `gstat` plugin is status-only and runs as a separate child process, and `git-shell` execs real git. The same build is also **the first shell with an fzf-compatible finder compiled in** — `arb --fzf` honors `FZF_DEFAULT_OPTS`, `FZF_DEFAULT_OPTS_FILE` and fzf's flag surface in-process, where zsh's key bindings, fzf-tab, fzf.fish and PSFzf all spawn the fzf binary. Elvish is the nearest miss: its histlist and location modes are real in-process fuzzy filtering, but they are shell-internal UI, not a finder that can filter a pipeline.

### [`Read the Docs`](https://menketechnologies.github.io/zshrs/index.html) &middot; [`Reference`](https://menketechnologies.github.io/zshrs/reference.html) · [`Coverage Report`](https://menketechnologies.github.io/zshrs/report.html) · [`strykelang`](https://github.com/MenkeTechnologies/strykelang) · [`fusevm`](https://github.com/MenkeTechnologies/fusevm) · [`compsys`](src/compsys/)

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
- [\[0x0D\] Native Rust Plugins](#0x0d-native-rust-plugins)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

zshrs replaces `fork + exec` with a persistent worker thread pool, compiles every command to [fusevm](https://github.com/MenkeTechnologies/fusevm) bytecodes, and **persists compiled chunks only in rkyv shards** under `~/.zshrs/images/` (single-directory rule — every zshrs file lives under `$ZSHRS_HOME` / `~/.zshrs/`; see [`docs/DAEMON.md`](docs/DAEMON.md)). Beside that tree, **`catalog.db` and related SQL views are read-only mirrors** for inspection (`dbview`, ad-hoc SQL): daemon-hydrated, **never authoritative for cache hit/miss or execution**. They are not a second shell cache. **`history.db`** holds history only — it is unrelated to bytecode caching. The result: shell startup, command dispatch, globbing, completion, and autoloading are all faster by orders of magnitude.

```text
                 [ THE MENKE-TECH REVOLUTIONARY FLYWHEEL ]

                 ┌──────────────────────────────────────┐
                 │       UNIFIED METAMORPHIC FRONT      │
                 │  zshrs  •  8 Bourne Family Dialects  │
                 └──────────────────────────────────────┘
                                    │
                                    ▼
                 ┌──────────────────────────────────────┐
                 │         FUSEVM EXECUTION CORE        │
                 │  Tiered Cranelift JIT / AOT Compiler │
                 └──────────────────────────────────────┘
                                    │
                                    ▼
                 ┌──────────────────────────────────────┐
                 │        SECURE HARDWARE PORTAL        │
                 │ Pure Rust Safety • Zero Memory CVEs  │
                 └──────────────────────────────────────┘
```

The **8 Bourne-family dialects** are the emulation drop-ins `--zsh`, `--bash`, `--ksh`, `--mksh`, `--pdksh`, `--sh`/`--posix`, `--dash`, and `--ash` (the C-shell `--csh` mode is separate). All eight compile through the same fusevm core, and each is verified against its **real reference shell** by the parity matrix in [`tests/emulation_parity.rs`](tests/emulation_parity.rs). zsh mode is additionally cross-checked against real `zsh` by a **differential fuzz harness** ([`bins/parity-fuzz.rs`](bins/parity-fuzz.rs)) that runs thousands of grammar-driven, seed-replayable snippets through both shells and flags any stdout/exit divergence.

---

## [0x01] INSTALL

```sh
# Via Homebrew tap (auto-bumped by each release)
brew tap MenkeTechnologies/menketech
brew install zshrs        # core: zshrs + zd
# OR
brew install zshrs-all    # umbrella: zshrs + zd + zshrs-recorder + zshrs-daemon

# From crates.io
cargo install zshrs

# From source — lean build, pure shell, no stryke dependency
git clone https://github.com/MenkeTechnologies/zshrs
cd zshrs && cargo build --release
# binary: target/release/zshrs

# Set as login shell
sudo sh -c 'echo "$(which zshrs)" >> /etc/shells'
chsh -s "$(which zshrs)"
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
| `git` (via [`zshrs-native`](https://github.com/MenkeTechnologies/zshrs-native)) | fork + exec /usr/bin/git | **Builtin** — zvcs linked in, zero fork |
| `fzf` (via [`zshrs-native`](https://github.com/MenkeTechnologies/zshrs-native)) | fork + exec the fzf binary | **Builtin** — arb's finder linked in, zero fork |
| `**/*.rs` | Single-threaded `opendir` | Parallel `walkdir` per-subdir on pool |
| `*(.x)` qualifiers | N serial `stat` calls | One parallel metadata prefetch |
| `rehash` | Serial `readdir` per PATH dir | Parallel scan across pool |
| `compinit` | Synchronous fpath scan | Background fpath scan on the worker pool |
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
                          Src/lex.c)  Src/parse.c)  ~10.6k LOC)
Script file (first)  ──► lex::zshlex ──► parse::parse ──► ZshCompiler ──► VM::run() ──► persist rkyv shard
Script file (cached) ──► index.rkyv + mmap shard ──► deserialize Chunk ──► VM::run()
                         (no lex, no parse, no compile)
Autoload function    ──► autoloads.rkyv ──► deserialize Chunk ──► VM::run()
                         (first call in a process compiles the definition
                          file and writes the chunk through; later processes
                          skip lex+parse+compile entirely. Each entry is
                          stamped with the definition file's mtime + length,
                          so an edited function recompiles.)
```

Measured on `_git` (424 KB of shell, the largest completer in common use):
first `git <tab>` in a process **1.06 s → 0.56 s** once the chunk is
cached, and decoding that cached 4.6 MB chunk costs **229 µs** against
**318 ms** to parse + compile the file. The cache is bypassed for
`ksh_autoload`-style bodies and for `autoload` without `-U`, where the
compiled program is not a function of the file's bytes alone.

The write-through fills one entry per function actually called. To
compile the whole corpus up front — so the FIRST `ls -<TAB>` of a fresh
install is already an O(1) shard probe:

```zsh
zshrs --prewarm-autoloads            # every dir on $fpath
zshrs --prewarm-autoloads DIR ...    # just these
zd prewarm [DIR ...]                 # same, via the daemon
```

`zshrs-recorder` runs the pass at the end of every recording (skip it
with `--no-prewarm`), which is where it belongs: the parser walks
process-global lexer state, so this must never run beside a live ZLE.
Entries already current are skipped by mtime + length, so a re-run after
installing one plugin costs one `stat` per completer. Budget roughly
**6× the source size** — a 13k-completer directory compiles to 165 MB in
35 s (debug build).

Enabling the JIT is not the same as being compiled by it. `zshrs --tiers script.zsh` runs the script and then asks fusevm's own predicates — `is_block_eligible`, `block_jit_is_compiled`, `trace_is_compiled`, `find_jit_region` — which tier took each chunk, reporting the script body and every function body it dispatched. Chunks that reach neither tier list the op kinds responsible, so the output is a diagnosis (what to make native next) rather than a verdict.

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

`pmap` / `pgrep` / `peach` run their body through the shared fusevm bytecode VM with the `{}` placeholder substituted as `${=__zshrs_p_arg__}` (matching zsh `SH_WORD_SPLIT` on literal substitution). The body is parsed and compiled to a fusevm `Chunk` ONCE before the input is iterated; the per-iteration loop sets the param via `setsparam`, acquires a VM from `fusevm::VMPool` (preserves Vec capacities across iterations), runs the cached chunk, and releases the VM back to the pool. `unsetparam` runs once at the end. No fork, no per-iteration parse/compile, no per-iteration VM allocation.

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

The `{ … }` body is a zshrs syntax extension — zsh cannot parse a bare `}`
as an argument. The lexer captures the span between the braces as raw
source, so the body keeps its own redirections, pipelines and
here-documents, and nothing in it is expanded until the advice fires.
`zshrs --zsh` turns the extension off and rejects the form exactly as
`/bin/zsh` does; `intercept before git 'code'` works in every mode.

Bound while advice runs: `$INTERCEPT_NAME`, `$INTERCEPT_ARGS`,
`$INTERCEPT_CMD`, plus `$INTERCEPT_MS`, `$INTERCEPT_US` and
`$INTERCEPT_STATUS` for `after`.

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

[zle]                      # fish-ported editor engines, all default off
autosuggest = true
syntax_highlight = true
history_search = true
autopair = true

[provenance]               # value-lineage engine; default true, and inert
enabled = true             # until `provenance -m NAME` arms it
track_all = false          # true = arm every parameter and every shell
                           # function with no `-m` at all
```

---

## [0x07] RKYV CACHE LAYOUT

Compiled bytecode and plugin/autoload payloads live in **rkyv** under `~/.zshrs/images/`:

| Path | Purpose |
|------|---------|
| **`index.rkyv`** | Top-level index: fq_name → shard id, generation, byte offset |
| **`images/{hash8}-*.rkyv`** | Mmap-ready shards (system, completions, plugins, scripts, `.zshrc`, …) |
| **`autoloads.rkyv`** | One compiled definition program per autoloaded function, keyed by name and stamped with the resolved fpath directory, a SHA-256 of the exact definition text, and the identity of the `zshrs` binary that compiled it |
| **`scripts.rkyv`** | One compiled chunk per script file, keyed by path + mtime |

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
| `provenance` | Value lineage — where a parameter's bytes came from and every bytecode op that touched them, each stamped with file, line and wall clock; shell functions carry the same chain ([`docs/PROVENANCE.md`](docs/PROVENANCE.md)) |

`provenance` (ported from strykelang's `mark` / `provenance` / `unmark`) answers
"where did this value come from?" for a running shell — an origin plus the op
chain the bytecode actually executed. No other shell records this: `typeset -p`
answers what a value *is*, `set -x` / `PS4` answer what *ran*, and every
provenance or taint-tracking system that answers the lineage question does it
from outside the shell.

```console
$ cat build.zsh
provenance -m ARCHIVE
REPORT=$(date +%Y-%m-%d)
ARCHIVE=${REPORT}.tar.gz
tar czf $ARCHIVE .
provenance ARCHIVE
$ zshrs build.zsh
ARCHIVE
  origin: cmdsubst "date +%Y-%m-%d" (build.zsh:2, 2026-08-18 11:22:03.908)
  ops:
     1. concat     "2026-08-18" ".tar.gz"                   build.zsh:3              11:22:03.908
     2. assign     ARCHIVE "2026-08-18.tar.gz"              build.zsh:3              11:22:03.908
     3. expand     $ARCHIVE "2026-08-18.tar.gz"             build.zsh:4              11:22:03.909
     4. exec       tar argv[2]                              build.zsh:4              11:22:03.909
```

Every row carries where and when it happened — file and line, the local
clock to the millisecond, and the enclosing shell function when the op
ran inside one (at its line in the file the function was *defined* in).

The chain covers the parameter's whole life, not one value: reassignment
appends an op and drops nothing, and a value arriving with its own lineage is
spliced in under an `origin` op.

Shell functions have chains of their own — definition site, every
redefinition, every call at the caller's line, and the `unfunction` that
ended it. `provenance -m NAME` arms whichever the name actually is, so a
function needs no extra flag; `-f` forces the function reading when a
parameter of the same name would otherwise win, and reads the chain
back:

```console
$ provenance -m greet          # arm it first — nothing is recorded until you do
$ greet; greet
$ provenance greet             # or `provenance -f greet`
greet()
  origin: function greet { MSG="hi $1" } (greet.zsh:2, 2026-08-18 11:48:01.139)
  ops:
     1. call       greet()                                  greet.zsh:3              11:48:01.139
     2. call       greet()                                  greet.zsh:4              11:48:01.140
```

The origin and every `redefine` carry the function's body (collapsed to one
line, truncated with `…`), so the chain shows what the body was and what it
was changed to. A `call` op carries the arguments the call was made with — `greet world`
records `greet(world)` — so repeated calls to one function stay
distinguishable; empty or whitespace-bearing arguments are single-quoted and
long lists are truncated with `…`.

Reading a name that was never armed is an error, not an empty chain —
`provenance -f greet` on its own answers `not tracked: greet()`.

Nothing is recorded until `provenance -m` arms it — or until something
turns on track-everything mode, which arms every parameter write and
every function with no `-m` at all: `provenance -a` at runtime,
`[provenance] track_all = true` in `~/.zshrs/zshrs.toml` from startup, or
`ZSHRS_PROVENANCE_ALL=1` in the environment. Self-rewriting parameters
(`LINENO`, `RANDOM`, `status`, the positionals, …) stay out of it, and
4096 auto-armed names is the ceiling. `[provenance] enabled = false` (or
`ZSHRS_PROVENANCE=0`) refuses arming altogether.

### Unit Test Framework (port of [`strykelang`](https://github.com/MenkeTechnologies/strykelang))

| Builtin | Description |
|---------|-------------|
| `zassert_eq` / `zassert_ne` | Equality / inequality |
| `zassert_ok` / `zassert_err` / `zassert_true` / `zassert_false` | Truthiness |
| `zassert_gt` / `zassert_lt` / `zassert_ge` / `zassert_le` | Numeric ordering |
| `zassert_match` | Regex match |
| `zassert_contains` | Substring containment |
| `zassert_near` | Float approximate equality (epsilon) |
| `zassert_dies` | Passes when given shell command exits non-zero |
| `ztest_skip` | Mark current assertion skipped |
| `ztest_run` / `run_tests` | Print summary, roll counters into totals |
| `zshrs --ztest [paths]` | Worker-pool runner — fork-on-receive (one persistent worker per CPU, dispatches `test_*` / `t_*` files under `t/` or `tests/`) |
| `zshrs --ztest-worker` | Persistent worker subprocess (JSON over stdin/stdout) |

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
m[name]=Alice; m[role]=eng        # set
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

### Runnable demos

[`examples/demos/*.zsh`](examples/demos/) — 375 self-contained scripts, every
one pinned to the same zshrs binary that runs CI. Sixteen batches:

| Range | Theme |
|---|---|
| 01–30 | shell fundamentals (arithmetic, arrays, assoc, control flow, fn/recursion, brace + parameter expansion, parameter flags, heredocs, command/process subst, pipes, printf, traps, IFS, anon fn, positional args, typeset, pattern match) |
| 31–60 | data structures (stack, queue, set ops), sorting (bubble/insertion/selection/counting), search (binary), classic algorithms (matrix mul, roman, hanoi, collatz, happy, armstrong, perfect, rot13, atbash, GCD/LCM), zsh idioms (CSV, env, file tests, date, read loop, exit codes, atoi, mapfile) |
| 61–85 | zsh C-feature demos — each cites `Src/*.c`: `:h:t:r:e:a:A:s` modifiers (`Src/hist.c`), parameter flags `(M)(P)(Q)(V)(j)(s)(o)(O)(u)(L)(U)(C)(l)(r)` (`Src/subst.c paramsubst`), glob qualifiers `(.)(/)(@)(N)(om)(oL)` (`Src/glob.c`), extended glob `^pat ~ # ## (alt\|alt) **/*` (`Src/pattern.c`), associative-array advanced ops (`Src/params.c` PM_HASHED), array set ops `:\|`/`:*`, pattern-filter `${arr:#pat}` / `(M):#`, typeset -i base, `print -aC` columnar (`Src/builtin.c bin_print`), `print -P` prompt escapes (`Src/prompt.c`), `zparseopts` (`Src/Modules/zutil.c`), `zsh/mathfunc` (`Src/Modules/mathfunc.c`), `zsh/datetime` (`Src/Modules/datetime.c`), `setopt local_options`, `eval` + dynamic dispatch (`Src/builtin.c bin_eval`), anonymous fns (`Src/exec.c is_anonymous_function_name`), compound defaults, advanced brace expansion (`Src/glob.c bracecomplete`), history-style word modifiers, complex split/join, mini calc REPL |
| 86–110 | advanced runtime patterns — `setopt` exhaustive (`Src/options.c`), `read -A -d` (`Src/builtin.c bin_read`), printf format dispatch, `[[ … =~ … ]]` + `$match` (`Src/cond.c cond_match` + `Src/Modules/regex.c`), `type`/`whence`/`which` (`Src/builtin.c bin_whence`), `hash` cmd cache (`Src/builtin.c bin_hash`), C-style for with multi-counter (`Src/parse.c`), 2D-assoc emulation, case alternation + fallthrough `;&` (`Src/exec.c execcase`), fd redirection + `&>` + `tee` (`Src/exec.c addfd`), strict mode `set -euo pipefail`, variable indirection `(P)` + `eval`, coreutils builtins (anti-fork extension), negative+ranged indexing, zsh-features summary, subshell-vs-group scope, function introspection (`functions[]` assoc), EXIT/ERR/ZERR traps, arithmetic edge cases, dispatch table via assoc, 3-stage pipelines + `pipefail`, eval metaprogramming, globsubst/`=cmd`/nullglob, boolean truth tables, `getopts`+`until`+`repeat`+`time`+`break N` |
| 111–135 | extension + utility patterns — `let` builtin (`Src/builtin.c bin_let`), assignment forms (`Src/exec.c addvars`), `typeset -T` tied colon-arrays (`Src/params.c` PM_TIED), `local -x -g -i -r -a -A` modifiers, greedy `##`/`%%` strips (`Src/subst.c`), conditional numeric ops `-eq`/`-gt` (`Src/cond.c cond_val`), capture-aware replacement, recursive `**/` globs (`Src/glob.c`), background `&` + `wait` (`Src/jobs.c`), UTF-8 string handling, mini-cat/mini-grep/mini-wc (pure-zsh coreutils), URL encode/decode, JSON pretty-printer, XML entity escape, string trim/pad/center, CSV writer with proper quoting, assoc serialize/deserialize, INI file parser, `emulate -L sh\|ksh` (`Src/options.c bin_emulate`), ksh-style `@()/+()/!()` patterns (`Src/pattern.c`), `zstyle` context store (`Src/Modules/zutil.c bin_zstyle`), `compdef` completion signatures (`Src/Zle/compsys.c`), `bindkey` keymap API (`Src/Zle/zle_keymap.c`) |
| 136–160 | systems + algorithms + apps — `path[]` tied to `$PATH` (`Src/params.c` PM_TIED), named pipes via `mkfifo`, file-lock via `mkdir`-atomic, env manipulation deep, `kill -USR1/USR2` signal handling (`Src/signals.c`), ANSI 256-color + `print -P %F`, calculator engine over `$((…))`, todo-list CRUD, BFS over adjacency-list graph, finite state machine via transition map, topological sort over DAG, pomodoro timer w/ `EPOCHREALTIME`, inventory system, append-only event log with filter/aggregate, LRU cache w/ eviction, sorted-output priority queue, Bloom filter w/ 3-hash, trie (prefix tree), Levenshtein edit distance (DP), line-level + word-level diff, `{{var}}` template renderer, observer pattern w/ subscriber registry, deterministic `$RANDOM` (`Src/params.c randomgetfn` / seed + Fisher-Yates shuffle), bank-account ledger w/ journal, self-introspecting capabilities report |
| 161–185 | utilities + meta — `pushd/popd/dirs` directory stack (`Src/builtin.c bin_pushd`), `umask` + `ulimit` (`Src/builtin.c bin_umask`/`bin_ulimit`), `${(q)}` / `${(qq)}` / `${(qqq)}` / `${(qqqq)}` quoting flags (`Src/subst.c`), `${(z)}` shell-words split (`Src/lex.c`), `print -aC -N -f -P` advanced flags (`Src/builtin.c bin_print`), `(#i)`/`(#a)`/`(#m)` pattern flags (`Src/pattern.c`), pure-zsh `mini_find` w/ -type/-name predicates, mini Makefile-style build w/ dep DAG, Markdown→plain-text stripper, regex tester driver, moving avg + peak detection + cumulative sum + trend, password strength scoring, anagram finder via canonical-form grouping, integer-to-English-words (0..999999), ASCII charts (horizontal + vertical + histogram), Conway's Game of Life (4 generations), days-between calculator w/ leap-year + Zeller's congruence, DFS maze generator w/ seeded `$RANDOM`, lottery simulator w/ match histogram, multi-month calendar printer, 6 FizzBuzz styles + reverse-FizzBuzz + counts-in-1..100, progress bars + braille spinners, word-frequency w/ top-N + once-only + filtering, cron-expression matcher (`* */N 0`), final 185-demo recap |
| 186–210 | parsers + apps + meta — `alias`/`-g`/`-s` forms (`Src/builtin.c bin_alias`), `xxd`-style hex dump, IPv4 parser + validator + subnet math, HTTP status code lookup + classifier, ANSI escape stripper, retry-with-exponential-backoff, memoize wrapper w/ stats, log rotation w/ size threshold, URL parser (scheme/user/host/port/path/query/fragment), 4 toy hash functions (poly + djb2 + FNV-1a + Adler-32), Base64 encoder w/ url-safe variant, full RFC4180 CSV parser w/ quoted fields + escaped quotes + embedded newlines, naive YAML key:value parser, 256-color + truecolor palettes, milestone banner (200th demo), unit converter (length/weight/temp/time), expression tokenizer, subcommand dispatcher CLI, string-interpolation patterns, zsh-specific scripting idioms, advanced assoc-array iteration (sort/filter/partition/transform), TTL cache w/ GC, recursive directory walker w/ emoji + size + extension stats, map/filter/reduce composable pipeline, quine + reflective `${functions[name]}` + mutual recursion |
| 211–235 | apps + games + introspection — CSV→Markdown table, Markdown table renderer w/ alignment, SSH config parser (Host blocks), chess board renderer from FEN, Tic-Tac-Toe (3 scripted games + win detect), card deck (Fisher-Yates shuffle + poker classify), number-guess (binary-search vs random over 100 games), quiz game w/ scoring + grades, Mad-libs template engine, expense tracker w/ category totals + percentages, `set -x` xtrace + `$PS4` customization (`Src/builtin.c bin_set`), `$PS1`/`$PS2`/`$PS3`/`$PS4`/`$RPROMPT` config + expansion (`Src/prompt.c`), `$funcstack`/`$functrace` call-stack introspection (`Src/exec.c`), git-log parser (commit/author/date/subject), Nginx-log analyzer (status + IP + method + bytes), todo w/ categories + priorities + due dates, open-addressing hash table impl, RPN stack machine (ADD/MUL/DUP/SWAP/DROP), multi-field array search + filter + group-by, menu-driven app w/ sub-menus, text adventure (rooms + exits + path), time tracker w/ project totals + bar chart, word-chain game (last-letter→first-letter), persistent KV-store mock (load/save assoc), grand-finale 235-demo banner |
| 236–260 | hooks + cryptography + grids + parsers — `precmd`/`preexec`/`chpwd` hooks via `add-zsh-hook` (`Src/Functions/Misc/add-zsh-hook`), `autoload -Uz` from `fpath` (`Src/builtin.c bin_functions`), Dijkstra shortest-path on weighted graph A-F, Sudoku validator (rows/cols/3x3 blocks), Lights Out 5×5 puzzle w/ toggle propagation, Hangman game w/ 7-stage ASCII gallows, famous number sequences (Fibonacci/Catalan/Lucas/Bell/triangular/pentagonal/hexagonal/factorial), Sierpinski triangle (Pascal mod 2 + bitwise + carpet), Mandelbrot set ASCII (fixed-point math, 60×24), TOML parser (sections + key/value + dotted lookup), .env-file parser w/ quoted + comments + `export` prefix, shebang detector (env-style + direct-path classification), charset validator (ASCII/hex/b64/UUID/email/url/ident), whitespace normalizer (strip/collapse/expand/unexpand/EOL/squash), shopping cart w/ tax + discount + inventory check, Vigenère cipher (encrypt/decrypt/identity/tableau), Caesar cipher + ROT13 + brute-force, word-search 8-dir solver w/ flat-string grid, ASCII 7-segment digital clock, IPv6 parser (expand `::` + compress longest zero-run), recipe-unit converter (fractions + scaling + vol/mass), memory-match concentration grid, monoalphabetic substitution cipher w/ keyed alphabet + frequency analysis, Boggle 4×4 DFS solver w/ 52-word dict, final 260-demo banner v3 |
| 261–285 | crypto + graphs + games + zsh hooks — prime factorization (trial division + canonical form), Miller-Rabin probabilistic primality w/ deterministic witnesses, extended Euclidean + modular inverse + RSA-toy, A* pathfinding on ASCII grid w/ Manhattan heuristic, Kruskal's MST w/ union-find + path compression, Prim's MST grown from start, Floyd-Warshall all-pairs shortest paths, Bellman-Ford + negative-cycle detect, N-Queens count + render (n=1..7 vs OEIS A000170), 15-puzzle slide + inversion-count solvability, Towers of Hanoi w/ animated render + 2ⁿ-1 verification, Markdown→HTML single-pass tokenizer (headings/bold/italic/code/lists/links/fenced), HTTP request parser (method/path/query/headers/body/cookies), log format auto-detector (Apache CLF/JSON/syslog/nginx/logfmt), CSV inner+outer join + aggregation, Blackjack dealer-17 + hand-value w/ Ace soft/hard, dice probability + Yahtzee pattern classifier + χ² fairness, Rock-Paper-Scissors 6-strategy round-robin tournament, XOR cipher + frequency analysis + Hamming key-length probe, One-Time Pad w/ key-reuse weakness demo, periodic/precmd/preexec/chpwd hooks (`Src/init.c periodic_sched_cmd` + add-zsh-hook), positional params + `getopts` deep dive (`Src/params.c $argv` + `Src/builtin.c bin_getopts`), trap matrix (EXIT/ERR/ZERR/USR1/USR2/INT/TERM/HUP via `Src/signals.c install_handler`), atomic file write via tmp+rename + mkdir-lock + toy CAS, grand finale 285-demo banner v4 |
| 286–310 | trees + games + strings + zsh internals — segment tree (range sum + point update + 200-query stress), Fenwick BIT tree (prefix sum + inversion count via O(n log n)), KMP string matching (failure table + period detect + naive cross-check), Rabin-Karp rolling-hash w/ collision verification, Manacher's longest palindrome O(n) + palindromic-substring count, reservoir sampling Algorithm R (Fisher-Yates cross-check), probabilistic skip list (level distribution + contains), suffix array + Kasai LCP (longest repeated substring + unique-substring count + binary-search lookup), word ladder BFS over 80-word dict (cat→dog chains), Soundex phonetic hash (Robert/Ashcraft/Tymczak classics), Minesweeper flood reveal w/ adjacency histogram, Mastermind w/ black/white peg scoring + color-frequency analysis, Tic-tac-toe minimax (terminal-state scoring), Conway's Life animated multi-pattern (glider/blinker/block), 🎉 demo 300 milestone banner, URL template (RFC 6570: `{?q}`/`{+base}`/`{#anchor}`/`{/path}`), mini SQL SELECT parser + in-memory executor (WHERE/ORDER BY/LIMIT), `print -P`/`-v`/`-aC`/`-z`/`-l`/`-N` flags (`Src/builtin.c bin_print`), unalias/unset/unhash/unfunction w/ `-m` pattern flag (`Src/builtin.c bin_unhash`), `typeset -m`/`-i`/`-F`/`-T`/`-A` deep dive (`Src/builtin.c bin_typeset`), zle widget definitions + bindkey + keymaps + `BUFFER`/`CURSOR` (`Src/Zle/zle_main.c`), compdef + `_arguments` + zstyle contexts (`Src/Zle/compsys.c`), graph density study (Kruskal MST cost vs density% trials), finite state machine DSL (turnstile/traffic light/vending/TCP), grand finale 310-demo banner v5 |
| 311–335 | trees + DP + zsh deep dives — iterative BST (insert/contains/inorder via stack), AVL rotation logic (LL/RR/LR/RL cases), Bloom filter v2 + union/intersection, double-ended queue (sliding-window max + BFS), ring buffer w/ overwrite, IPv4 subnet calc (CIDR/mask/broadcast/contains), MAC address parser + OUI vendor DB, file checksums (Adler-32/DJB2/FNV-1a/sum16/XOR), anagram solver (canonical sort + grouping + phrases), leet speak basic + advanced + random, pig latin encode + reverse, zsh HISTFILE parser (extended `: ts:dur;cmd` + raw + freq stats), SSH known_hosts parser (plain + hashed + duplicate detect), brace expansion deep dive (numeric/alpha/nested/product/padded), `print -r/-R/-D/-aC/-P/-v/-u/-s/-z/-N/-m/-o/-O/-i/-e/-E` flags (`Src/builtin.c bin_print`), `compinit` + completion lifecycle (`Src/Zle/compsys.c`), `extended_glob` deep dive (`^`/`~`/`#`/`##`/`<a-b>`/`(#i)`/`(#a)` + qualifiers, `Src/pattern.c`), assoc array `(@kv)` flag + 2-dim + invert + merge (`Src/params.c PM_HASHED` + `Src/subst.c paramsubst`), max subarray (Kadane + circular + stock profit), LIS + LCS + edit distance DP, 0/1 knapsack + fractional comparison, coin change (min + count ways + reconstruction), topological sort (Kahn's + cycle detection w/ partial-order recovery), LRU cache (doubly-linked list + hash + working-set sim), grand finale 335-demo banner v6 |
| 336–360 | history math + parsers + ciphers + zsh metadata — Roman numeral encoder/decoder + validator, trie (insert/search/autocomplete/count_prefix/longest_common_prefix), Z-function + Z-based substring search + period detection, longest common substring DP + brute-force cross-check, longest palindromic subsequence DP + min-insertions, Nim game + XOR theorem + optimal-play strategy proof, peg solitaire + greedy solver, RFC 2822 date parser (email-header format) + epoch conversion, ISO 8601 parser (date/time/duration/week/ordinal), Pollard's rho factorization + Miller-Rabin small primality, continued fractions + sqrt approximation + golden ratio + Lagrange theorem, columnar transposition cipher + rail fence, color conversions (RGB↔HSL↔HSV + hex parse + 256-color mapping + luminance + WCAG contrast), `$ZSH_EVAL_CONTEXT` + `$FUNCNEST` + `$ZSH_SUBSHELL` (`Src/init.c eval_context` + `Src/exec.c` subshell counter), 🎉 demo-350 milestone banner, Sokoban small puzzle + push/wall semantics, text wrap (greedy + center + right + full-justify) + whitespace normalization, Unicode utilities (display width + byte count + JSON escape + hex escape), URL encode/decode RFC 3986 + URL parser + query string + form encoding, calendar (month grid + year overview + Zeller's congruence + leap year), disjoint-set union (path compression + grid islands + friendship circles), priority queue (min-heap + Dijkstra application + top-N stream), `$funcstack`/`$funcfiletrace`/`$functrace`/`$funcsourcetrace` (`Src/exec.c` func stack), comprehensive parameter expansion flags (`(U)`/`(L)`/`(C)`/`(j)`/`(s)`/`(o)`/`(O)`/`(q)`/`(qq)`/`(qqq)`/`(qqqq)`/`(M)`/`(P)` exhaustively, `Src/subst.c paramsubst`), grand finale 360-demo banner v7 |
| 361–367 | substantial functional demos (500+ LOC each) — comprehensive JSON parser (RFC 7159: tokenizer + AST + JSONPath + pretty-print), full XML parser (tags + attributes + CDATA + comments + entity decode + XPath subset), arithmetic expression evaluator (Shunting-yard tokenizer + RPN stack-based eval + variables + functions abs/min/max/sqrt), RFC 4180 CSV parser (state machine + quoted fields + embedded newlines + CRLF + custom delimiter + round-trip serializer), mini-Lisp interpreter (tokenizer + s-expression parser + closures + recursion + cond/let/lambda/define + lexical scope chain), Sudoku backtracking solver (row/col/3×3 block validation + algorithm reference), grand finale 367-demo banner v8 |
| 368–375 | codecs + interpreters + algorithms — Bencode encoder/decoder (BitTorrent wire format), Hamming(7,4) single-bit error-correcting block code, skyline problem (key-point outline from N building triples), Brainfuck interpreter (pure-zsh), Ackermann function (total non-primitive-recursive, Ackermann 1928 / Péter simplification), LZW (Lempel–Ziv–Welch) compression (the algorithm behind GIF + Unix `compress`), Elias gamma universal codes for positive integers, grand finale 375-demo banner v9 |

```bash
cargo build --bin zshrs
target/debug/zshrs --zsh examples/demos/10_fizzbuzz.zsh
cargo test --test examples_demos_ci          # full sweep, ~46s parallel
```

---

## [0x0A] COMPATIBILITY

- Full zsh script compatibility — runs existing `.zshrc`
- Full bash compatibility via emulation
- **Native fish-ported line-editor engines** (opt-in via `~/.zshrs/zshrs.toml` `[zle]` — bare `zshrs -f` stays byte-identical to `zsh -f` for parity): syntax highlighting (lexer-driven command/keyword/quote/redirection/path coloring with command-validity and file-existence checks; the engine is the fish port, the palette and word-level classification are `fast-syntax-highlighting`'s — command-type ladder, brackets-by-nesting-level pass, glob/variable/path/option/math/here-string/case styles, secondary theme inside `$(…)`, and the `autoload` / `source` / `printf` chromas — so the native pass renders the same as the plugin it replaces), history autosuggestions (ghost text, accept with →/End, word-wise accept with forward-word), prefix/substring up-arrow history search, and bracket/quote auto-pairing (zsh-autopair port) — ports of the fish-shell Rust engines that zsh-syntax-highlighting, zsh-autosuggestions, and zsh-history-substring-search recreate in script. Existing plugin config applies (`ZSH_HIGHLIGHT_STYLES`, `ZSH_AUTOSUGGEST_*`, `HISTORY_SUBSTRING_SEARCH_*`, `AUTOPAIR_*`); when enabled, the native engines stay authoritative even with the script plugins loaded (their config honored, their widgets subsumed); `ZSHRS_NATIVE_ZLE_FX=0` force-disables. Per-keystroke passes are wall-clock budgeted (`ZSHRS_ZLE_{HIGHLIGHT,AUTOSUGGEST}_BUDGET_MS`, default 8) so huge directories or PATHs can never lag typing
- Fish-style abbreviations
- **180+ builtins** (150 zsh + 23 coreutils + parallel primitives) — see the [Reference](https://menketechnologies.github.io/zshrs/reference.html) for the full catalog
- ZWC precompiled function support
- Glob qualifiers, parameter expansion flags, completion system
- zstyle, ZLE widgets, hooks, modules
- Per-shell emulation drop-ins: `--zsh`, `--bash`, `--ksh`, `--mksh`, `--pdksh`, `--sh`/`--posix`, `--dash`, and `--ash`. `--mksh` (MirBSD ksh) and `--pdksh` (Public Domain ksh) share the `--ksh` base; `--ash` (Almquist shell) is an alias of `--dash`. `--dash` is a strict Debian Almquist Shell mode — it applies the `sh` option presets and additionally rejects the zsh-only syntax dash has never had (`$'...'` ANSI-C quoting, `<<<` here-strings, `+=` compound assignment, `name=(...)` arrays, the `[[ ]]` reserved word, arith `**`/`,`, and `printf %q`) while using XSI `echo` — verified byte-for-byte against `/bin/dash`
- Drop-in fidelity beyond zsh's own `emulate`: the bare POSIX-family modes match the REAL shell, not zsh's approximation of it. `--sh`/`--dash`/`--ash` use XSI `echo` (backslash escapes interpreted without `-e`, as both `/bin/sh` flavours and `dash` do) where zsh's `emulate sh` sets `BSD_ECHO`, and `--bash` accepts bash's own `set -o` names — including the six zsh has no option for (`posix`, `errtrace`, `functrace`, `history`, `keyword`, `nolog`) — reporting them through `set -o`, `set +o` and `$SHELLOPTS`. Adding `--zsh` (`--sh --zsh`, `--ksh --zsh`) selects zsh-STYLE emulation instead: the option deltas a real zsh installs for `emulate sh` / `emulate ksh`, referenced against zsh itself.

### Test corpus parity

| Suite | Tests | Coverage |
|-------|-------|----------|
| `parity` | 47,009 | Differential assertions against real `zsh` — expansion, completion, builtins, modules, job control, diagnostics |
| `zsh_construct_corpus` | 396 | Every sh/zsh construct outside modules |
| `no_tree_walker_dispatch` | 174 | Behavioral pins for the no-tree-walker invariant |
| `zsh_corpus_via_new_pipeline` | 123 | Native lex+parse+ZshCompiler path |
| `zsh_parser_probe` | 87 | AST-shape probes for every construct |
| `compile_zsh_smoke` | 28 | Per-construct bytecode-level smoke |
| `tree_walker_absent` | 8 | Source-level absence checks (anti-regression) |
| `ztst_runner` | 70 files / 2,604 chunks | Real `.ztst` files from upstream zsh — see [Compatibility measurement](#compatibility-measurement) for the chunk-level score |
| **Total** | **47,825** | excluding ztst chunks; see below for the ztst figure |

### Compatibility measurement

Three independent measurements, all re-runnable. Numbers below were taken on
macOS aarch64 against `zsh 5.9.2` (`/opt/homebrew/bin/zsh`) as the oracle.

**Differential parity suite** — [`tests/parity/`](tests/parity) is the largest
measurement here: 47,009 hand-written assertions run against real `zsh`,
comparing stdout, exit status and (for diagnostics) stderr.

```
cargo test --test parity
```

Latest full run: **46,985 passed, 6 failed, 18 ignored**. Reading those two
small numbers honestly:

- Of the 6 failures, 3 were environmental — `binary_parity` spawns
  `target/debug/zshrs-daemon`, which a concurrent `cargo clean` had removed;
  rebuilding it (`cargo build -p zshrs-daemon`) restores 4/4. One more passes
  in isolation and only fails under heavy parallel load. That leaves **2 real**:
  a coproc job-table listing and dash-mode `getopts` state.
- The 18 ignored are documented gaps, each `#[ignore]`d with a citation and an
  entry in [`docs/BUGS.md`](docs/BUGS.md). Five are reference-version skew
  rather than defects: the port follows the vendored C tree in
  `~/forkedRepos/zsh` (`5.9.0.3-test`), which carries changes absent from the
  released 5.9.x line — `time` on builtins, `:S` history-style substitution,
  dotted parameter namespaces, `typeset -n`. Verified against BOTH 5.9 and
  5.9.2: each rejects all four, so zshrs is ahead of the oracle, not wrong.
  The rest are open bugs.
- One case (`probe_b_row_149`) flips run to run with machine load; it is a
  genuine residual job-control race, not a flaky test.

**Differential fuzz** — [`bins/parity-fuzz.rs`](bins/parity-fuzz.rs) generates
seed-replayable snippets per grammar mode, runs them through both shells, and
flags any stdout/exit divergence. Every case is re-confirmed 3x (`--verify 3`)
so a single load-induced flake cannot register as a divergence.

```
parity-fuzz --mode <mode> --count 300 --verify 3 --timeout-ms 20000
```

| | zsh oracle | emulation targets |
|---|---|---|
| Cases | 22,200 (74 modes) | 2,100 (7 shells) |
| Divergences | **27 (0.12%)** | 709 (33.8%) |
| Modes/targets at zero | **71 of 74** | -- |

All residual zsh-mode divergence is in three modes, and each has a single
identified root cause:

| Mode | Count | Root cause |
|---|---|---|
| `unicode` | 18 | `unsetopt multibyte` is not honoured. zsh drops to BYTE semantics (so `[[:alpha:]]` matches one byte of a multibyte character); zshrs stays in character mode. All 18 cases set the option. |
| `quote` | 8 | zsh's token bytes `0x84`-`0xA1` (`Src/lex.c:38` `ztokens`) are stored as Rust `char`s, so a real codepoint in U+0084-U+00A1 is indistinguishable from a token. Verified boundary: U+009F/A0/A1 mangle under `(V)`/`(q)`/`(qqqq)`, U+00A2 and above are clean. C avoids this by Meta-escaping bytes. |
| `zmv` | 1 | `zmv -W` no-match error reports the unconverted pattern and the shell's own name (`zsh:1: no matches found: *.*`) instead of zsh's converted pattern and function context (`zmv:239: no matches found: (*).(*)`). |

Two further notes on reading this table:

- `prompt` shows 0 divergences but a reproducible ~10-12% timeout rate at a
  20 s limit; the harness reports timeouts separately and does not attribute
  a side.
- Run the sweep DETACHED (`nohup`, CI, a supervisor) and the shell inherits
  `SIGQUIT` as `SIG_IGN`. zsh records that as an ignored trap and lists
  `trap -- '' QUIT` (`c:Src/init.c:1444-1445`), and an inherited `SIGHUP`
  ignore clears the `HUP` option (`c:1451-1452`). zshrs reached neither,
  because its `-c` and script-file dispatch bypass `init_signals`; the
  `trap` mode reported ~114/300 in that context and 0 in a foreground
  shell. Fixed by porting just those two records onto the bypass paths
  (`src/extensions/startup_signals.rs`) — NOT by calling `init_signals`
  there, which also installs C's SIGCHLD handler and regresses pipeline
  and job reaping badly (measured: `pipeline` 0 -> 139). Converging those
  dispatch paths onto `zsh_main` is still the structurally correct fix.
  The recording is ZSH-only and gated off in the drop-in modes: with both
  signals inherited-ignored, `trap` prints the QUIT line in zsh, both
  signals SIG-prefixed in bash, and nothing at all in dash/ksh/sh.

Per emulation target, out of 300 each: `ksh` 163, `mksh` 130, `pdksh` 130,
`bash` 112, `sh` 80, `dash` 47, `ash` 47. The emulation modes are held to a
weaker contract than zsh mode -- exit-status *sign* rather than exact code --
because the reference shells disagree with each other on exact codes.

**Upstream ztst corpus** -- the `.ztst` files shipped with zsh, run by
[`tests/ztst_runner.rs`](tests/ztst_runner.rs):

```
cargo test --test ztst_runner -- --nocapture   # per-file chunk tallies
```

**1,798 of 2,604 chunks pass (69.0%)**; 776 fail, 30 skip. 23 of the 70 files
are 100% green.

Read the cargo line with care: the 70 file-level tests report `ok`
unconditionally. `run_ztst` prints `NOTE: N failures … (baseline mode — not
failing CI)` and never asserts, so "70 passed, 0 failed" measures nothing. The
chunk tally above — summed from the per-file lines under `--nocapture` — is the
real figure.

The failures are far more structural than 776 independent bugs. Six files score
zero, and five of them for a single reason:

| Source | Failed | Passed | Cause |
|---|---|---|---|
| `Y03arguments.ztst` | 97 | 0 | prep TIMEOUT |
| `X05zleincarg.ztst` | 95 | 0 | prep TIMEOUT |
| `X02zlevi.ztst` | 95 | 0 | prep TIMEOUT |
| `Y02compmatch.ztst` | 58 | 0 | prep TIMEOUT |
| `Y01completion.ztst` | 33 | 0 | prep TIMEOUT |
| `V01zmodload.ztst` | 40 | 0 | prep exits 1 |
| `D04parameter.ztst` | 48 | 197 | genuine per-chunk gaps |
| `B02typeset.ztst` | 28 | 60 | genuine per-chunk gaps |
| `V10private.ztst` | 28 | 14 | genuine per-chunk gaps |
| `K01nameref.ztst` | 19 | 112 | genuine per-chunk gaps |

**378 chunks — 49% of every failure — are blocked by the 10 s prep budget, not
by shell behaviour.** Those files' `%prep` runs `comptestinit`, which spawns the
shell under test inside `zsh/zpty` and runs `compinit` in it; against a debug
build that needs 60-90 s. At `ZTST_TIMEOUT_MS=90000` the `Y03arguments` prep
completes. Past it, the first pty completion round-trip then hangs, which wedges
the file's shell and marks every later chunk "not run" — so this whole block is
one bug plus one budget, not 378 defects. `zsh/zpty` itself is sound: `-r -m`,
`-w` and `-t` all behave, and a spawn → `compinit` → `zle -C` sequence completes
through a pty.

The `D04`/`B02`/`V10`/`K01` shape is the opposite and the honest measure of
remaining debt: broadly working, with a real tail.

A second, independent accounting agrees. `tests/gen/ztst_failures.rs` pins 1,292
individual chunks as `#[ignore]`d known gaps; running them
(`cargo test --test ztst_runner -- --ignored`) gives **546 passed, 746 failed**.
The 746 lines up with the 776 counted from the file-level tally, from a
completely different code path, so the 69% figure is not an artifact of how one
of them counts.

It also means **546 of the 1,292 pins (42%) are stale** — they were pinned
against gaps that have since been fixed and nobody un-pinned them. Read the pin
list as an upper bound on debt, not a measurement of it; the chunk tally is the
measurement.

The gap between the fuzz figure and the ztst figure is intentional and worth
reading carefully: the fuzzer samples the grammar it knows how to generate,
while ztst samples what zsh's own authors thought worth testing. The ztst
number is the one that predicts daily-driver trust.

---

## [0x0B] ARCHITECTURE

The codebase is **structurally divided into ported code vs extensions**, with the boundary mechanically enforced by `tests/port_purity.rs`. Bots, contributors, and humans all read [`docs/PORT.md`](docs/PORT.md) before writing a single line.

```
                  ┌────────────────────────────────────────────────────────────────┐
                  │                        zshrs workspace                         │
                  │             3 crates · 832 .rs files · 880k lines              │
                  ├──────────────────────────────────────────┬─────────────────────┤
                  │      src/ (466 .rs — runtime crate)      │  vendor/fish/ (157) │
                  │  ┌────────────────────────────────────┐  │  reader / line edit │
                  │  │  src/ported/  (106 — STRICT PORT)  │  │  syntax highlight   │
                  │  │  every .rs ↔ a real Src/<x>.c file │  │  autosuggest        │
                  │  │  every fn carries `/// Port of …`  │  │  abbreviations      │
                  │  │  enforced by tests/port_purity.rs  │  │  env dispatch       │
                  │  │  builtins/ · zle/ · modules/ ·     │  │  history backend    │
                  │  │  hist · jobs · params · pattern ·  │  │  process control    │
                  │  │  signals · glob · subst · math ·   │  │  event system       │
                  │  │  prompt · utils · init · …         │  ├─────────────────────┤
                  │  └────────────────────────────────────┘  │  parse + lex now    │
                  │  ┌────────────────────────────────────┐  │  live IN-RUNTIME    │
                  │  │  src/extensions/  (94 — NON-PORT)  │  │  (folded from the   │
                  │  │  features zsh C does NOT have:     │  │  old parse crate)   │
                  │  │  AOT · plugin/script/autoload      │  ├─────────────────────┤
                  │  │  cache · fish_features · worker    │  │  daemon/ (41 .rs)   │
                  │  │  pool · zwc · arith_compiler ·     │  │  zshrs-daemon — IPC │
                  │  │  hooks · keymaps · widgets ·       │  │  · HTTP · OpenAPI · │
                  │  │  daemon_presence · log · …         │  │  fsnotify · cache · │
                  │  └────────────────────────────────────┘  │  zsource/zhistory/  │
                  │  ┌────────────────────────────────────┐  │  zjob builtins      │
                  │  │  src/recorder/  (1 — feature gate) │  ├─────────────────────┤
                  │  │  AOP intercept; #[cfg(recorder)]   │  │ compsys folded into │
                  │  │  → zero bytes in default binary    │  │ runtime (was its    │
                  │  └────────────────────────────────────┘  │ own crate)          │
                  ├──────────────────────────────────────────┴─────────────────────┤
                  │    bins/  (5 .rs — 3 shipped + bench-autoload, parity-fuzz)    │
                  │     zshrs            zshrs-recorder         zd                 │
                  │     (default)        (--features recorder)  (--features zd)    │
                  ├────────────────────────────────────────────────────────────────┤
                  │                      fusevm (bytecode VM)                      │
                  │            235 opcodes · fused superinstructions · JIT         │
                  └────────────────────────────────────────────────────────────────┘
```

### Directory rule (PORT.md)

| Directory | Rule | Enforcement |
|-----------|------|-------------|
| `src/ported/` | **Strict 1:1 port.** Every `.rs` mirrors a real upstream zsh `Src/<x>.c`; every top-level `fn` carries `/// Port of <cname>() from Src/<file>.c:NNNN`; no invented helpers; **directory and file set FROZEN** (106 files, no new files allowed). | `tests/port_purity.rs` |
| `src/compsys/ported/` | **1:1 mirror of zsh's `Completion/` tree.** Engine functions (`Base/{Completer,Core,Utility,Widget}`, `Zsh/Context`, plus engine-only entries in `Unix/Type`, `Zsh/Type`, `Zsh/Command`, `Unix/Command`, and top-level `compinit`/`compdump`) are ported to Rust as `<name>.rs` and carry a `Port of _<NAME>` header citing the upstream shell source. End-user shell completers (`*/Command`, `Zsh/Function`, end-user type files) are **copied as-is alongside** the Rust ports — same dir layout, same filename, no `.rs` extension — and dispatched via the `_call_function` bridge. Current coverage: **991 upstream files mirrored, 247 engine .rs ports**. Regenerate the coverage report with `scripts/gen_compsys_port_report.py`. | per-fn tests; doc-comment shell-source citations; `gen_compsys_port_report.py` |
| `src/extensions/` | **Non-port only.** Features zsh C demonstrably does *not* have. Must not duplicate or shadow any port. | `port_purity` exempts the 1:1 file rule for this directory only |
| `src/recorder/` | **Feature-gated.** Every symbol `#[cfg(feature = "recorder")]`; deleted by rustc when off. | `Cargo.toml` `required-features = ["recorder"]` on the `zshrs-recorder` bin |

---

## [0x0C] EDITOR INTEGRATION

zshrs ships an **LSP server** and **DAP debug adapter** built into the
binary, plus a **JetBrains IDE plugin** that drives both.

### CLI flags

```sh
zshrs --lsp                  # LSP server over stdio
zshrs --dap HOST:PORT        # DAP debugger; TCP connect-back to IDE listener
zshrs --dap                  # DAP debugger over stdio (executable-spawned clients)
zshrs --dump-reflection      # JSON dump of builtins / keywords / options
zshrs --dump-plugins         # JSON dump of every sourced plugin grouped
                             # by manager (zinit / oh-my-zsh / prezto /
                             # antidote / antigen / zplug / loose);
                             # feeds the IDE's External Libraries view
zshrs --docs <name>          # render the LSP hover card for <name>
zshrs --fmt [-w] [-t] [-i N] [FILE…]
                             # format zsh source: block-structure
                             # reindent + idiomatic spacing (dedupe
                             # runs; a;b → a; b, a&&b → a && b,
                             # a|b → a | b, cmd& → cmd &, x;; → x ;;,
                             # f () → f(), bare then/do join onto
                             # the opener as '; then'/'; do' per the
                             # zsh + OMZ style guides), heredoc-safe,
                             # idempotent. Indent defaults to 4
                             # spaces per level; editor tabSize and
                             # CLI -i N override explicitly.
                             # stdin→stdout with no files; -w rewrites
                             # in place; -i sets indent width (default
                             # 4); -t indents with tabs
```

All six flags dispatch from `bins/zshrs.rs` into `src/extensions/lsp.rs`,
`src/extensions/dap.rs`, and `src/extensions/plugin_cache.rs`. The LSP and
DAP modules are dependency-free additions
(no `lsp-server` / `lsp-types` / `dap-types` crates) — Content-Length
framing + JSON-RPC are hand-rolled on top of `serde_json` to keep the
default build lean.

### LSP capabilities (`zshrs --lsp`)

| Capability                          | Trigger                                  |
|-------------------------------------|------------------------------------------|
| `completion`                        | builtins, keywords, options, special vars, in-file functions **plus live compsys matches** — the same `_git` / `_docker` / `_ssh` completers a Tab press drives, so `git ch` completes `checkout` and `git checkout ` lists branches (see `docs/IN_EDITOR_COMPSYS_COMPLETION.md`). Writing a completer is covered too: inside an `_arguments` spec, the action field offers completer names and the inline action forms (`(list)`, `((val\:desc))`, `->state`) |
| `hover`                             | markdown cards for builtins / keywords / options / special vars |
| `definition` / `references`         | function names declared in the open document |
| `documentHighlight`                 | same scan as references                  |
| `documentSymbol`                    | `function foo`, `foo()`, `alias`, `local`/`typeset`/`export` |
| `foldingRange`                      | `{ … }` / `do … done` / `case … esac` blocks + ≥3 `#` comment runs |
| `rename` (with `prepareRename`)     | word-boundary aware replace across document |
| `semanticTokens/full`               | comment / string / number / keyword / variable / function classes |
| `formatting`                        | full syntax-aware reindent + idiomatic spacing (`src/extensions/fmt.rs`, same engine as `--fmt`): if/fi, do/done, case arms, `{ }`, `( )`, `[[ ]]`, continuations; whitespace dedupe and canonical `;` `&&` `\|\|` `\|` `&` `;;` spacing with quote/`${…}`/fd-redirect/glob-pattern guards; heredoc bodies verbatim |
| `publishDiagnostics`                | brace + block matching, unclosed strings, lights up on `didOpen` / `didChange` / `didSave` |

Trigger characters for completion: `$`, `{`, `-`, `:`. Optional
`ZSHRS_LSP_LOG=<path>` env var dumps every request/response for debugging.

Compsys completion reads the recorded environment from the canonical
rkyv shard (`~/.zshrs/images/*-recorder.rkyv`) — `compdef` map, `fpath`,
autoload stubs, `zstyle` — so it needs `zshrs-recorder` to have run at
least once; without a shard the LSP still completes everything else in
the table above. Completers that shell out (`git for-each-ref` behind
`git checkout <tab>`) run with their subprocess killed at the request
deadline, and never inherit the editor's stdin/stdout.

### DAP capabilities (`zshrs --dap [HOST:PORT]`)

Two transports, same DAP server: `--dap HOST:PORT` connects back to the IDE's
TCP listener (JetBrains; keeps stdout free for the script), while bare `--dap`
serves DAP over stdio for clients that spawn the adapter as an executable
(e.g. VS Code's `DebugAdapterExecutable`).

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
- **Hand-rolled lexer** with 45 independently-themeable color slots
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

## [0x0D] NATIVE RUST PLUGINS

Every shell before zshrs runs plugins as interpreted scripts. zshrs is
the first JIT-compiled Unix shell, and the first that **hosts plugins written
in a native compiled language** (Rust), loaded at runtime with no
recompile of the shell. A plugin is an ordinary `cdylib` the shell
`dlopen`s through a stable, versioned **C ABI** (the
[`znative`](znative/) crate). Nothing about Rust's unstable
layout, allocator, or panic ABI crosses the boundary — only
`#[repr(C)]` data.

```rust
// src/lib.rs — crate-type = ["cdylib"], deps: znative
use znative::{declare_plugin, Args, Host};
use std::os::raw::c_int;

fn rhello(host: &Host, args: &Args) -> c_int {
    let pwd = host.getvar("PWD").unwrap_or_default();
    host.print(&format!("hello, {} (pwd={pwd})\n", args.rest().join(" ")));
    0
}

declare_plugin! {
    name: "hello",
    version: "0.1.0",
    builtins: { "rhello" => rhello },
}
```

```bash
cargo build                              # → target/debug/libhello.dylib (.so on Linux)

# Inside zshrs:
zmodload -R ./target/debug/libhello.dylib   # load
zmodload -R                                  # list loaded plugins
rhello world                                 # native command, no fork, no interpreter
zmodload -uR hello                           # unload by name
```

Plugin commands resolve after real builtins and shell functions, before
PATH lookup — the same slot zsh uses for `zmodload -ab` autoloaded
builtins. The host API a plugin can call back through: `print`, `eval`
(run shell code), `getvar` / `setvar` (shell scalars), `getfunction` /
`addfunction` (read a function's deparsed body / define one — also
deparse-as-a-service), and `register_builtin`.

### Installing plugins with `znative`

`zmodload -R` is the primitive; **`znative`** is the package manager built
on it. One line per plugin in your `.zshrc` — it installs on the first
shell start (clone → `cargo build --release` for native plugins →
`zmodload -R`) and then loads from a content-addressed store, zero-network,
every start after:

```bash
znative load MenkeTechnologies/zshrs-forgit    # native (Rust) plugin, self-installing
znative load zsh-users/zsh-autosuggestions     # zsh script plugins work too
```

Sources also take `github:owner/repo`, `git+URL@ref`, and `path:DIR`.
Manage the store with `znative list` / `info` / `update` / `remove`. Full
reference: [`docs/ZNATIVE.md`](docs/ZNATIVE.md).

Plugins can also provide **native (Rust) completions** — a
`completions:` block in `declare_plugin!` wires a Rust generator into
zsh's completion system (compsys), so `mycmd <TAB>` runs your Rust code to
produce candidates. See [`examples/plugin-complete/`](examples/plugin-complete/).

**Porting existing zsh plugins:** [`docs/PORTING_ZSH_PLUGIN.md`](docs/PORTING_ZSH_PLUGIN.md)
is a construct-by-construct zsh→Rust guide, worked end-to-end on
[**forgit**](examples/plugin-forgit/) (the git+fzf plugin, ported
command-for-command: `ga glo gd gcf gclean gss gcp grh gi`) and — for the
harder *self-reentrant* case — [**git-fuzzy**](examples/plugin-git-fuzzy/)
(fzf calling back into the tool per keystroke, with a `--listen` live-reload
watcher).

### vs bash / zsh native plugins

Native runtime plugins aren't new — bash `enable -f file.so name` and zsh
`zmodload` both load native builtins. The difference is the *interface*:

|                    | script plugin (`.zsh`) | bash `enable -f` / zsh module | zshrs Rust plugin |
| ------------------ | ----------------- | ----------------- | ----------------- |
| Artifact           | `.zsh` (interpreted) | `.so` built in the shell's tree | `.dylib`/`.so` `cdylib` |
| Build against      | nothing (sourced) | the shell's **private** internal headers | published `znative` crate |
| Stable ABI         | n/a | **none** — welded to one shell build, no version gate | versioned, load-time checked |
| Distribution       | a file to source | rebuild per shell release | one crates.io SDK crate |
| Speed              | interpreted | native | native |
| Third-party viable | yes (scripts only) | **rare** (needs shell source) | **yes** (`cargo add`) |

bash and zsh load native code only through their **internal** C APIs — you
compile against the shell's private headers, with no stable ABI and no
version gate, so a plugin is welded to one build and can crash a
mismatched one. That's why neither has meaningful third-party native
plugins. zshrs exposes a stable, published, versioned ABI (`cargo add
znative`), version-gated at load — first shell to make its
native-plugin interface an independently-published ABI package instead of
its own build-tree internals.

A runnable example lives in [`examples/plugin-hello/`](examples/plugin-hello/).
Full guide: [`docs/PLUGINS.md`](docs/PLUGINS.md); package manager: [`docs/ZNATIVE.md`](docs/ZNATIVE.md).

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
OpenAPI surface, flat-text history + sibling FTS5 index,
bytecode-level value lineage as a builtin) is
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
> MenkeTechnologies.

Ideas can't be copyrighted so this is an ask, not an
MIT-enforced clause; honoring it keeps the legacy traceable.
See [CREATORS.md § Legacy](CREATORS.md#legacy) +
[§ Attribution expectation](CREATORS.md#attribution-expectation)
for the full list + suggested forms.
