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

The first Unix shell to compile to bytecodes and execute on a purpose-built virtual machine with fused superinstructions. Since the Bourne shell at Bell Labs in 1970, every Unix shell has been an interpreter. zshrs is the first to be a compiler. A drop-in zsh replacement written in Rust — **658k+ lines, 490 source files** across a 2-crate workspace (`zshrs` runtime + `zshrs-daemon`; `compsys` was folded into the runtime), with the runtime split into a **strict 1:1 port directory** (`src/ported/` — 106 files, every fn maps to a real `Src/<x>.c` zsh function, enforced by `tests/port_purity.rs`), **a non-port extensions directory** (`src/extensions/` — 42 files, features zsh C does not have: AOT, daemon coordination, plugin/script/autoload caches, fish-style autosuggest/abbrev/highlight, persistent worker pools, ZWC byte-code helpers), and a feature-gated recorder (`src/recorder/`). **193 ZLE widgets registered** in `IWIDGET_NAMES` (history navigation, vi find/repeat/marks, undo/redo, isearch, yank-pop, shell-aware word motion, region/visual mode, text objects, completion menu, $zle_highlight parsing), 47 fish-ported builtins, persistent worker pool, AOP intercept, **rkyv**-backed bytecode images (mmap hot path; the only shell bytecode cache), **read-only SQLite mirrors** beside them for `dbview` / SQL inspection only (no cache semantics), and full zsh compatibility.

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
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

zshrs replaces `fork + exec` with a persistent worker thread pool, compiles every command to [fusevm](https://github.com/MenkeTechnologies/fusevm) bytecodes, and **persists compiled chunks only in rkyv shards** under `~/.zshrs/images/` (single-directory rule — every zshrs file lives under `$ZSHRS_HOME` / `~/.zshrs/`; see [`docs/DAEMON.md`](docs/DAEMON.md)). Beside that tree, **`catalog.db` and related SQL views are read-only mirrors** for inspection (`dbview`, ad-hoc SQL): daemon-hydrated, **never authoritative for cache hit/miss or execution**. They are not a second shell cache. **`history.db`** holds history only — it is unrelated to bytecode caching. The result: shell startup, command dispatch, globbing, completion, and autoloading are all faster by orders of magnitude.

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
                          Src/lex.c)  Src/parse.c)  ~10.6k LOC)
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
| `zsh_corpus_via_new_pipeline` | 124 | Native lex+parse+ZshCompiler path |
| `no_tree_walker_dispatch` | 160 | Behavioral pins for the no-tree-walker invariant |
| `compile_zsh_smoke` | 28 | Per-construct bytecode-level smoke |
| `tree_walker_absent` | 8 | Source-level absence checks (anti-regression) |
| `zsh_parser_probe` | 87 | AST-shape probes for every construct |
| `ztst_runner` | 70 | Real `.ztst` files from upstream zsh |
| **Total** | **869** | All green on the new (default) pipeline |

---

## [0x0B] ARCHITECTURE

The codebase is **structurally divided into ported code vs extensions**, with the boundary mechanically enforced by `tests/port_purity.rs`. Bots, contributors, and humans all read [`docs/PORT.md`](docs/PORT.md) before writing a single line.

```
                  ┌────────────────────────────────────────────────────────────────┐
                  │                       zshrs workspace                          │
                  │             2 crates · 490 .rs files · 658k+ lines             │
                  ├──────────────────────────────────────────┬─────────────────────┤
                  │  src/ (288 .rs — runtime crate)           │  vendor/fish/ (157) │
                  │  ┌────────────────────────────────────┐   │  reader / line edit │
                  │  │  src/ported/  (106 — STRICT PORT)  │   │  syntax highlight   │
                  │  │  every .rs ↔ a real Src/<x>.c file │   │  autosuggest        │
                  │  │  every fn carries `/// Port of …`  │   │  abbreviations      │
                  │  │  enforced by tests/port_purity.rs  │   │  env dispatch       │
                  │  │  builtins/ · zle/ · modules/ ·     │   │  history backend    │
                  │  │  hist · jobs · params · pattern ·  │   │  process control    │
                  │  │  signals · glob · subst · math ·   │   │  event system       │
                  │  │  prompt · utils · init · …         │   ├─────────────────────┤
                  │  └────────────────────────────────────┘   │  parse + lex now    │
                  │  ┌────────────────────────────────────┐   │  live IN-RUNTIME    │
                  │  │  src/extensions/  (42 — NON-PORT)  │   │  (folded from the   │
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
                  │            224 opcodes · fused superinstructions · JIT         │
                  └────────────────────────────────────────────────────────────────┘
```

### Directory rule (PORT.md)

| Directory | Rule | Enforcement |
|-----------|------|-------------|
| `src/ported/` | **Strict 1:1 port.** Every `.rs` mirrors a real upstream zsh `Src/<x>.c`; every top-level `fn` carries `/// Port of <cname>() from Src/<file>.c:NNNN`; no invented helpers; **directory and file set FROZEN** (106 files, no new files allowed). | `tests/port_purity.rs` |
| `src/compsys/ported/` | **1:1 mirror of zsh's `Completion/` tree.** Engine functions (`Base/{Completer,Core,Utility,Widget}`, `Zsh/Context`, plus engine-only entries in `Unix/Type`, `Zsh/Type`, `Zsh/Command`, `Unix/Command`, and top-level `compinit`/`compdump`) are ported to Rust as `<name>.rs` and carry a `Port of _<NAME>` header citing the upstream shell source. End-user shell completers (`*/Command`, `Zsh/Function`, end-user type files) are **copied as-is alongside** the Rust ports — same dir layout, same filename, no `.rs` extension — and dispatched via the `_call_function` bridge. Current coverage: **991 upstream files mirrored, 128 engine .rs ports**. Regenerate the coverage report with `scripts/gen_compsys_port_report.py`. | per-fn tests; doc-comment shell-source citations; `gen_compsys_port_report.py` |
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
| `completion`                        | builtins, keywords, options, special vars, in-file functions |
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
> MenkeTechnologies.

Ideas can't be copyrighted so this is an ask, not an
MIT-enforced clause; honoring it keeps the legacy traceable.
See [CREATORS.md § Legacy](CREATORS.md#legacy) +
[§ Attribution expectation](CREATORS.md#attribution-expectation)
for the full list + suggested forms.
