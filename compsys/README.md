# compsys

**Zsh-compatible completion system for zshrs.**

The completion engine that powers zshrs Tab completion — ported from zsh's `Src/Zle/complist.c` and `Src/Zle/compresult.c` with SQLite FTS5 indexing, menuselect state machine, and zstyle configuration.

Part of the [zshrs](https://github.com/MenkeTechnologies/zshrs) workspace.

## Features

- **SQLite FTS5 completion cache** — instant fuzzy prefix search across all PATH executables and autoload functions
- **MenuState** — full zsh menuselect state machine: grid navigation with column memory, undo stack, incremental search, interactive filtering
- **MenuKeymap** — configurable key-to-action mapping for menuselect, matching zsh's `bindkey -M menuselect`
- **CompletionGroup** — grouped completions with headers, colors, and descriptions (zstyle `list-colors`)
- **compinit** — parallel fpath scan with rayon, stores function bodies and bytecodes in SQLite
- **compadd/compdef** — zsh-compatible completion registration
- **zstyle** — cascading style configuration for completion formatting and behavior

## Architecture

```
Tab keypress
    │
    ▼
ZshrsCompleter::complete()
    │
    ├── Command position → SQLite FTS5 prefix search (executables + builtins)
    ├── File position → readdir + glob
    └── Option position → compsys completion functions
            │
            ▼
    CompletionGroup[] → MenuState::set_completions()
            │
            ▼
    MenuState::render() → colored grid with group headers
            │
            ▼
    MenuKeymap::lookup(key) → MenuAction → MenuState::process_action()
```

## Key Types

| Type | Description |
|------|-------------|
| `CompsysCache` | SQLite database: autoloads, comps, services, executables, zstyles |
| `MenuState` | Grid position, viewport, mode (normal/interactive/search), undo stack |
| `MenuKeymap` | Key sequence → widget name → `MenuAction` mapping |
| `MenuAction` | Accept, Cancel, Up, Down, Left, Right, PageUp, PageDown, Search, Undo, ... |
| `MenuResult` | Continue, Accept(String), Cancel, Redisplay, UndoRequested |
| `MenuRendering` | Rendered lines with ANSI colors, selection highlight, status |
| `CompletionGroup` | Name, matches, explanations, sorted/unsorted, flags |
| `Completion` | str, description, display override, suffix, flags |

## SQLite Tables

| Table | Purpose |
|-------|---------|
| `autoloads` | Function bodies + compiled bytecodes for instant loading |
| `comps` | Command → completion function mapping (`_comps` hash) |
| `services` | Command → service mapping (`_services` hash) |
| `patcomps` | Pattern → function mapping (`_patcomps` hash) |
| `executables` | PATH executables with FTS5 index |
| `zstyles` | Cascading style configuration |
| `shell_functions` | Named shell functions |

## License

MIT — Part of the [zshrs](https://github.com/MenkeTechnologies/zshrs) project.
