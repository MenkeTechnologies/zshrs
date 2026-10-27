```
  ██████╗ ██████╗ ███╗   ███╗██████╗ ███████╗██╗   ██╗███████╗
 ██╔════╝██╔═══██╗████╗ ████║██╔══██╗██╔════╝╚██╗ ██╔╝██╔════╝
 ██║     ██║   ██║██╔████╔██║██████╔╝███████╗ ╚████╔╝ ███████╗
 ██║     ██║   ██║██║╚██╔╝██║██╔═══╝ ╚════██║  ╚██╔╝  ╚════██║
 ╚██████╗╚██████╔╝██║ ╚═╝ ██║██║     ███████║   ██║   ███████║
  ╚═════╝ ╚═════╝ ╚═╝     ╚═╝╚═╝     ╚══════╝   ╚═╝   ╚══════╝
```

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

### `[RUST PORTS OF ZSH'S COMPLETION/ FUNCTIONS FOR ZSHRS]`

> *"Tab is the most powerful key on the keyboard."*

`src/compsys/` holds the Rust ports of zsh's shell-level completion
system — the `Completion/` function tree (`_main_complete`,
`_arguments`, `_files`, `_git`, …) — plus the router that decides,
per call, whether a port or a shell function runs. The C completion
engine underneath (`compcore`, `compadd`, `compset`, menuselect,
`zstyle`) is not here; it is ported under `src/ported/zle/` and
`src/ported/modules/zutil.rs`.

### [`zshrs`](https://github.com/MenkeTechnologies/zshrs) · [`fusevm`](https://github.com/MenkeTechnologies/fusevm) · [`strykelang`](https://github.com/MenkeTechnologies/strykelang)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Dispatch path](#0x01-dispatch-path)
- [\[0x02\] Modules](#0x02-modules)
- [\[0x03\] SQLite mirror tables](#0x03-sqlite-mirror-tables)
- [\[0x04\] Completion functions](#0x04-completion-functions)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

- **Ports, not reimplementations.** Every `_NAME.rs` under `ported/`
  is the Rust shadow of one upstream `Completion/` shell function,
  stored next to the upstream file it ports.
- **Router-arbitrated.** A port runs only where zsh would have run the
  stock function: a shell function already defined under that name, or
  a non-stock file for it earlier in `$fpath`, wins. A plugin
  `CompFn` registered with `zmodload -R` wins over both.
- **Switchable.** `[compsys] backend = "shell"` in
  `$ZSHRS_HOME/zshrs.toml` (default `~/.zshrs/zshrs.toml`) turns the
  built-in ports off; every name then autoloads from `$fpath` as in zsh.
- **rkyv for state, SQLite for inspection.** Completion state
  (`compdef` map, autoload stubs, zstyles) comes from the daemon's
  rkyv shards (see [`docs/DAEMON.md`](../../docs/DAEMON.md)); compiled
  autoload chunks are cached in `~/.zshrs/autoloads.rkyv`
  (`src/extensions/autoload_cache.rs`). The SQLite file `cache.rs`
  writes is a read-only mirror for `dbview` / SQL and is never
  consulted on the Tab path.

---

## [0x01] DISPATCH PATH

```
Tab (complete-word, registered by compinit's `zle -C`)
  → ported::zle::zle_tricky::completecall / completeword
    → docomplete → docompletion → runhookdef(COMPLETEHOOK)
      → ported::zle::compcore::do_completion → callcompfunc
        → ported::exec::doshfunc(synth shfunc, body_runner)
          → ported::exec::run_function_body("_main_complete")
            → compsys::router::dispatch_compsys
                ├── plugin CompFn (zmodload -R)            if registered
                ├── router::try_rust_dispatch → run_port   Rust port
                └── None → autoload + fusevm body          shell function
  → port body calls siblings by name (shared::call_compfn), each
    through the same router; `compadd` lands in
    ported::zle::complete::bin_compadd → compcore::addmatches
  → menuselect: ported::zle::complist::domenuselect
```

Link-by-link C parity, and the observable `$zsh_eval_context`
difference this path produces, are in
[`docs/COMPLETION_DISPATCH.md`](../../docs/COMPLETION_DISPATCH.md).

---

## [0x02] MODULES

| File | Role |
|------|------|
| `mod.rs` | Module declarations; `COMPSYS_FN_NAMES` (inventory surfaced by the LSP reflection dump and `docs/reference.html`); re-exports of `ported::compinit` and `zpwr_colors` |
| `router.rs` | Port-vs-shell arbitration. `dispatch_compsys`, `try_rust_dispatch`, `run_port`, `is_intercepted`; `rust_compsys_lookup` is the name → port table. zshrs-original, no C counterpart |
| `in_editor.rs` | `complete_at(CompsysRequest) -> CompsysResponse` for the LSP: runs `_main_complete` on a dedicated shell thread bootstrapped from the recorder shard and captures `compadd` output. Design: [`docs/IN_EDITOR_COMPSYS_COMPLETION.md`](../../docs/IN_EDITOR_COMPSYS_COMPLETION.md) |
| `cache.rs` | `CompsysCache` — the SQLite mirror (see 0x03), its path (`$ZSHRS_HOME/compsys.db`), rebuild lock and binary-identity stamp |
| `zpwr_colors.rs` | zstyle / `LS_COLORS` parsing for completion list colors |
| `ported/` | The port tree (see 0x04) |

---

## [0x03] SQLITE MIRROR TABLES

Created by `cache.rs`. Read-only views for SQL and `dbview`; shell
behavior never depends on these rows for caching or dispatch.

| Table | Contents |
|-------|----------|
| `autoloads` | Autoload function names and source metadata |
| `comps` | Command → completion function (`_comps`) |
| `services` | Command → service (`_services`) |
| `patcomps` / `postpatcomps` | Pattern → function (`_patcomps` / `_postpatcomps`) |
| `keycomps` | Key → completion function (`compdef -k`) |
| `executables` | `$PATH` executables |
| `named_dirs` | Named directories |
| `shell_functions` | Shell function bodies |
| `zstyles` | Style patterns and values |
| `metadata` | Mirror bookkeeping |
| `cache` | Result cache |
| `fts_comps`, `fts_executables`, `fts_shell_functions` | FTS5 tables for prefix search from `dbview` |

---

## [0x04] COMPLETION FUNCTIONS

`ported/` mirrors the upstream `Completion/` tree, directory for
directory, with the upstream shell file and its Rust port side by
side (`Base/Core/_main_complete` + `Base/Core/_main_complete.rs`):

```
compsys/ported/
├── Base/{Completer,Core,Utility,Widget}/
├── Unix/{Command,Type}/
├── Zsh/{Command,Context,Function,Type}/
├── X/{Command,Type,Utility}/
├── AIX/ BSD/ Darwin/ Debian/ Linux/ Solaris/   {Command,Type}/
├── Cygwin/ Mandriva/ openSUSE/ Redhat/         Command/ (no ports yet)
├── compinit.rs  compdump.rs  compaudit.rs      ports of the top-level scripts
├── shared.rs                                   helpers shared by ports
└── mod.rs                                      #[path] declarations
```

Current counts:

```sh
find src/compsys/ported -name '_*.rs' | wc -l                  # ports
grep -c '=> Some(' src/compsys/router.rs                       # router arms
find src/compsys/ported -type f ! -name '*.rs' -name '_*' | wc -l   # upstream functions
```

### `compsys/ported/` is canonical-port code — same rule as `src/ported/`

- **Strict 1:1 mapping.** `_<NAME>.rs` ports the upstream shell
  function `_<NAME>`; module `_<NAME>`, public fn `_<NAME>`. Ports
  called by name from siblings split into a `_<NAME>` wrapper
  (`shared::call_compfn`) and a `_<NAME>_impl` body; the router arm
  must name `_<NAME>_impl`. `shared.rs` is the only non-port module.
- **Faithful semantics.** Every flag the shell function accepts must be
  honored. No "returns true to make the test pass" stubs — those
  failed an internal audit
  ([`docs/audits/PORT_STUBS.md`](../../docs/audits/PORT_STUBS.md)) and
  were replaced with full ports.
- **Shell-source citations.** Each port has a module-level
  `//! Port of …` header naming the upstream `Completion/…` file;
  per-line `sh:N` comments cite the shell source where the algorithm
  is non-obvious.
- **Tests prove non-fake.** Every port carries a `#[cfg(test)]` block
  that fails if the body is replaced with `return true`. Run
  `cargo test --lib compsys::ported`.
- **Layout mirrors zsh.** Module names stay flat
  (`compsys::ported::_command_names`) via
  `#[path = "Zsh/Type/_command_names.rs"]`-style attributes in `ported/mod.rs`.

Ad-hoc Rust-native code does not go in `ported/`: it lives in
`src/extensions/` or in the non-port files of `src/compsys/`
(`router.rs`, `in_editor.rs`, `cache.rs`, `zpwr_colors.rs`).

---

## [0xFF] LICENSE

MIT — Part of the [zshrs](https://github.com/MenkeTechnologies/zshrs) project. Copyright (c) 2026 [MenkeTechnologies](https://github.com/MenkeTechnologies)
