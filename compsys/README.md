```
  ██████╗ ██████╗ ███╗   ███╗██████╗ ███████╗██╗   ██╗███████╗
 ██╔════╝██╔═══██╗████╗ ████║██╔══██╗██╔════╝╚██╗ ██╔╝██╔════╝
 ██║     ██║   ██║██╔████╔██║██████╔╝███████╗ ╚████╔╝ ███████╗
 ██║     ██║   ██║██║╚██╔╝██║██╔═══╝ ╚════██║  ╚██╔╝  ╚════██║
 ╚██████╗╚██████╔╝██║ ╚═╝ ██║██║     ███████║   ██║   ███████║
  ╚═════╝ ╚═════╝ ╚═╝     ╚═╝╚═╝     ╚══════╝   ╚═╝   ╚══════╝
```

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

### `[ZSH-COMPATIBLE COMPLETION ENGINE FOR ZSHRS]`

> *"Tab is the most powerful key on the keyboard."*

The completion engine that powers [zshrs](https://github.com/MenkeTechnologies/zshrs) Tab completion — ported from zsh's `Src/Zle/complist.c` and `Src/Zle/compresult.c` with **rkyv**-mmap'd bytecode shards (the completion cache), read-only SQLite **mirrors** for SQL/`dbview` introspection only (no role in Tab cache hit/miss), a full menuselect state machine, and zstyle configuration. 27 source files, 23k+ lines of Rust.

### [`zshrs`](https://github.com/MenkeTechnologies/zshrs) · [`fusevm`](https://github.com/MenkeTechnologies/fusevm) · [`strykelang`](https://github.com/MenkeTechnologies/strykelang)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Architecture](#0x01-architecture)
- [\[0x02\] Features](#0x02-features)
- [\[0x03\] Key Types](#0x03-key-types)
- [\[0x04\] SQLite mirror tables](#0x04-sqlite-mirror-tables)
- [\[0x05\] Completion Functions](#0x05-completion-functions)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

compsys replaces zsh's C completion system with a Rust implementation whose **hot path is rkyv**: pre-compiled [fusevm](https://github.com/MenkeTechnologies/fusevm) chunks live in mmap'd shards under `~/.cache/zshrs/images/`, coordinated by `index.rkyv` (see [`docs/DAEMON.md`](../docs/DAEMON.md)). Where zsh scans `fpath` and reparses on every `compinit`, compsys consumes daemon-built images and falls through to compile-on-demand only on miss.

**SQLite** exists only as **read-only mirrors** (joinable rows, optional FTS materializations) for humans and `dbview`. Mirrors are hydrated from daemon state; they **do not define or influence** the shell completion cache.

- **rkyv mmap bytecode** — zero-copy load of completion / autoload chunks on the warm path (the cache)
- **SQLite mirrors** — SQL-side inspection only; not consulted for execution or cache semantics
- **MenuState** — full zsh menuselect state machine: grid navigation with column memory, undo stack, incremental search, interactive filtering
- **MenuKeymap** — configurable key-to-action mapping for menuselect, matching zsh's `bindkey -M menuselect`
- **CompletionGroup** — grouped completions with headers, colors, and descriptions (zstyle `list-colors`)
- **compinit** — parallel fpath scan with rayon; compiled output is rolled into **rkyv** shards
- **compadd/compdef** — zsh-compatible completion registration
- **zstyle** — cascading style configuration for completion formatting and behavior

---

## [0x01] ARCHITECTURE

```
Tab keypress
    │
    ▼
ZshrsCompleter::complete()
    │
    ├── Command position ──► rkyv-backed completion index
    │                        (executables + builtins; mmap hot path)
    ├── File position ──► readdir + glob
    └── Option position ──► compsys completion functions
            │                    (fusevm bytecodes from rkyv mmap)
            ▼
    CompletionGroup[] ──► MenuState::set_completions()
            │
            ▼
    MenuState::render() ──► colored grid with group headers
            │
            ▼
    MenuKeymap::lookup(key) ──► MenuAction ──► MenuState::process_action()
```

---

## [0x02] FEATURES

| Feature | Description |
|---------|-------------|
| Prefix completion | Sub-millisecond matching from **rkyv** / mmap'd tables (not from SQLite cache) |
| Menuselect grid | Arrow navigation, column memory, viewport scrolling |
| Incremental search | Type to narrow within menuselect |
| Interactive filter | Pattern-based filtering within completion list |
| Undo stack | Full undo/redo within menuselect |
| Group headers | Colored headers with match counts |
| `list-colors` | zstyle-configurable ANSI colors per completion |
| Bytecode autoloads | Completion functions pre-compiled to fusevm; **stored in rkyv** |
| Parallel compinit | Rayon-powered fpath scan and function compilation |
| `_arguments` | Full zsh `_arguments` spec parser and generator |

---

## [0x03] KEY TYPES

| Type | Description |
|------|-------------|
| `CompsysCache` | Optional handle to **read-only mirror** DB for tooling; bytecode and cache hits live in **rkyv** only |
| `MenuState` | Grid position, viewport, mode (normal/interactive/search), undo stack |
| `MenuKeymap` | Key sequence -> widget name -> `MenuAction` mapping |
| `MenuAction` | Accept, Cancel, Up, Down, Left, Right, PageUp, PageDown, Search, Undo, ... |
| `MenuResult` | Continue, Accept(String), Cancel, Redisplay, UndoRequested |
| `MenuRendering` | Rendered lines with ANSI colors, selection highlight, status |
| `CompletionGroup` | Name, matches, explanations, sorted/unsorted, flags |
| `Completion` | str, description, display override, suffix, flags |

---

## [0x04] SQLITE MIRROR TABLES

Daemon-hydrated **read-only** views for SQL and `dbview`. They duplicate metadata for human queries; **shell behavior never depends on these rows for caching or dispatch.** Authoritative compiled payloads are in **rkyv** shards (`image_path` / offsets per [`docs/DAEMON.md`](../docs/DAEMON.md)).

| Table | Purpose (mirror / inspection only) |
|-------|---------|
| `autoloads` | Names, source metadata, pointers into rkyv for bodies / chunks |
| `comps` | Command -> completion function mapping (`_comps` hash) |
| `services` | Command -> service mapping (`_services` hash) |
| `patcomps` | Pattern -> function mapping (`_patcomps` hash) |
| `executables` | PATH executables (optional FTS materialization on mirror) |
| `zstyles` | Cascading style configuration |
| `shell_functions` | Named shell functions |

---

## [0x05] COMPLETION FUNCTIONS

compsys ships with ported zsh completion functions organized by category:

```
compsys/functions/
├── Base/
│   ├── Completer/    _complete, _approximate, _expand, _history, ...
│   ├── Core/         _main_complete, _description, _tags, _wanted, ...
│   ├── Utility/      _arguments, _describe, _values, _multi_parts, ...
│   └── Widget/       _complete_help, _correct_word, _generic, ...
├── Unix/
│   └── Type/         _files, _directories, _path_files
└── Zsh/
    └── Command/      _command
```

---

## [0xFF] LICENSE

MIT — Part of the [zshrs](https://github.com/MenkeTechnologies/zshrs) project. Copyright (c) 2026 [MenkeTechnologies](https://github.com/MenkeTechnologies)
