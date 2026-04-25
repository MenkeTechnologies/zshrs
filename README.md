# zshrs

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**The first compiled Unix shell. The most powerful shell ever created.**

> *"No fork, no problems."*

A drop-in zsh replacement written in Rust. The first Unix shell to compile to bytecodes and execute on a purpose-built virtual machine with fused superinstructions. Since the Bourne shell at Bell Labs in 1970, every Unix shell has been an interpreter. zshrs is the first to be a compiler.

## Install

```sh
# Lean build — pure shell, full concurrent primitives, no stryke
cargo install --path .

# From source
git clone https://github.com/MenkeTechnologies/zshrs
cd zshrs && cargo build --release
# binary: target/release/zshrs

# Set as login shell
sudo sh -c 'echo ~/.cargo/bin/zshrs >> /etc/shells'
chsh -s ~/.cargo/bin/zshrs
```

## No-Fork Architecture

Every operation that zsh forks for runs in-process on a persistent worker thread pool:

| Operation | zsh | zshrs |
|-----------|-----|-------|
| `$(cmd)` | fork + pipe | In-process stdout capture via `dup2` |
| `<(cmd)` / `>(cmd)` | fork + FIFO | Worker pool thread + FIFO |
| `**/*.rs` | Single-threaded `opendir` | Parallel `walkdir` per-subdir on pool |
| `*(.x)` qualifiers | N serial `stat` calls | One parallel metadata prefetch |
| `rehash` | Serial `readdir` per PATH dir | Parallel scan across pool |
| `compinit` | Synchronous fpath scan | Background scan + bytecode compilation |
| History write | Synchronous `fsync` | Fire-and-forget to pool |
| Autoload | Read file + parse every time | Bytecode deserialization from SQLite |
| Plugin source | Parse + execute every startup | Delta replay from SQLite cache |

## Bytecode Compilation

Every command compiles to fusevm bytecodes:

```
Interactive command → Parser → ShellCompiler → fusevm::Op → VM::run()
Script file (first) → Parser → ShellCompiler → VM::run() → cache bytecodes in SQLite
Script file (cached) → SQLite → deserialize Chunk → VM::run() (no lex, no parse, no compile)
Autoload function   → SQLite → deserialize Chunk → VM::run() (microseconds)
```

## Concurrent Primitives

Full parallelism in the thin binary. No stryke dependency needed.

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

## AOP Intercept

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

## Worker Thread Pool

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

## SQLite Caching

Three databases power the shell:

- **compsys.db** — completions: autoloads with bytecodes, comps, services, PATH executables (FTS5)
- **history.db** — frequency-ranked, timestamped, duration, exit status per command
- **plugins.db** — plugin delta cache: functions, aliases, variables, hooks, zstyles, options

Browse without SQL:

```zsh
dbview                        # list tables + row counts
dbview autoloads _git         # single function: source, body, bytecode status
dbview comps git              # search completions
dbview history docker         # search history
```

## Exclusive Builtins

| Builtin | Description |
|---------|-------------|
| `intercept` | AOP before/after/around advice on any command |
| `intercept_proceed` | Call original from around advice |
| `async` / `await` | Ship work to pool, collect result |
| `pmap` | Parallel map with ordered output |
| `pgrep` | Parallel filter |
| `peach` | Parallel for-each, unordered |
| `barrier` | Run all commands in parallel, wait for all |
| `doctor` | Full diagnostic: pool metrics, cache stats, bytecode coverage |
| `dbview` | Browse SQLite caches without SQL |
| `profile` | In-process command profiling with nanosecond accuracy |

## Compatibility

- Full zsh script compatibility — runs existing `.zshrc`
- Full bash compatibility via emulation
- Fish-style syntax highlighting, autosuggestions, abbreviations
- 150+ builtins ported from zsh
- ZWC precompiled function support
- Glob qualifiers, parameter expansion flags, completion system
- zstyle, ZLE widgets, hooks, modules

## Documentation

- [HTML docs](https://menketechnologies.github.io/strykelang/zshrs.html)
- [stryke docs](https://menketechnologies.github.io/strykelang/)

## License

MIT — Copyright (c) 2026 MenkeTechnologies
