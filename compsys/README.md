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

The completion engine that powers [zshrs](https://github.com/MenkeTechnologies/zshrs) Tab completion — ported from zsh's `Src/Zle/complist.c` and `Src/Zle/compresult.c` with SQLite FTS5 indexing, a full menuselect state machine, and zstyle configuration. 27 source files, 23k+ lines of Rust.

### [`zshrs`](https://github.com/MenkeTechnologies/zshrs) · [`fusevm`](https://github.com/MenkeTechnologies/fusevm) · [`strykelang`](https://github.com/MenkeTechnologies/strykelang)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Architecture](#0x01-architecture)
- [\[0x02\] Features](#0x02-features)
- [\[0x03\] Key Types](#0x03-key-types)
- [\[0x04\] SQLite Tables](#0x04-sqlite-tables)
- [\[0x05\] Completion Functions](#0x05-completion-functions)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

compsys replaces zsh's C completion system with a Rust implementation backed by SQLite FTS5. Where zsh scans `fpath` directories and parses function files on every `compinit`, compsys pre-compiles completion functions to [fusevm](https://github.com/MenkeTechnologies/fusevm) bytecodes and stores them in SQLite. Prefix completion queries hit an FTS5 index instead of linear-scanning arrays.

- **SQLite FTS5 completion cache** — instant fuzzy prefix search across all PATH executables and autoload functions
- **MenuState** — full zsh menuselect state machine: grid navigation with column memory, undo stack, incremental search, interactive filtering
- **MenuKeymap** — configurable key-to-action mapping for menuselect, matching zsh's `bindkey -M menuselect`
- **CompletionGroup** — grouped completions with headers, colors, and descriptions (zstyle `list-colors`)
- **compinit** — parallel fpath scan with rayon, stores function bodies and bytecodes in SQLite
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
    ├── Command position ──► SQLite FTS5 prefix search
    │                        (executables + builtins)
    ├── File position ──► readdir + glob
    └── Option position ──► compsys completion functions
            │
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
| FTS5 prefix search | Sub-millisecond fuzzy matching across all completions |
| Menuselect grid | Arrow navigation, column memory, viewport scrolling |
| Incremental search | Type to narrow within menuselect |
| Interactive filter | Pattern-based filtering within completion list |
| Undo stack | Full undo/redo within menuselect |
| Group headers | Colored headers with match counts |
| `list-colors` | zstyle-configurable ANSI colors per completion |
| Bytecode autoloads | Completion functions pre-compiled to fusevm bytecodes |
| Parallel compinit | Rayon-powered fpath scan and function compilation |
| `_arguments` | Full zsh `_arguments` spec parser and generator |

---

## [0x03] KEY TYPES

| Type | Description |
|------|-------------|
| `CompsysCache` | SQLite database: autoloads, comps, services, executables, zstyles |
| `MenuState` | Grid position, viewport, mode (normal/interactive/search), undo stack |
| `MenuKeymap` | Key sequence -> widget name -> `MenuAction` mapping |
| `MenuAction` | Accept, Cancel, Up, Down, Left, Right, PageUp, PageDown, Search, Undo, ... |
| `MenuResult` | Continue, Accept(String), Cancel, Redisplay, UndoRequested |
| `MenuRendering` | Rendered lines with ANSI colors, selection highlight, status |
| `CompletionGroup` | Name, matches, explanations, sorted/unsorted, flags |
| `Completion` | str, description, display override, suffix, flags |

---

## [0x04] SQLITE TABLES

| Table | Purpose |
|-------|---------|
| `autoloads` | Function bodies + compiled bytecodes for instant loading |
| `comps` | Command -> completion function mapping (`_comps` hash) |
| `services` | Command -> service mapping (`_services` hash) |
| `patcomps` | Pattern -> function mapping (`_patcomps` hash) |
| `executables` | PATH executables with FTS5 index |
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
