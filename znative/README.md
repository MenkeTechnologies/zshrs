```
███████╗███╗   ██╗ █████╗ ████████╗██╗██╗   ██╗███████╗
╚══███╔╝████╗  ██║██╔══██╗╚══██╔══╝██║██║   ██║██╔════╝
  ███╔╝ ██╔██╗ ██║███████║   ██║   ██║██║   ██║█████╗  
 ███╔╝  ██║╚██╗██║██╔══██║   ██║   ██║╚██╗ ██╔╝██╔══╝  
███████╗██║ ╚████║██║  ██║   ██║   ██║ ╚████╔╝ ███████╗
╚══════╝╚═╝  ╚═══╝╚═╝  ╚═╝   ╚═╝   ╚═╝  ╚═══╝  ╚══════╝
```

[![Crates.io](https://img.shields.io/crates/v/znative.svg)](https://crates.io/crates/znative)
[![Docs.rs](https://docs.rs/znative/badge.svg)](https://docs.rs/znative)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

### `[NATIVE PLUGIN SDK FOR ZSHRS]`

> *"A shell plugin that runs at native speed — no interpreter, no re-source."*

## `[THE ABI]`

`znative` is the stable, versioned **C ABI** for writing native (Rust) plugins for [zshrs](https://github.com/MenkeTechnologies/zshrs) — the first JIT-compiled Unix shell, and the first that hosts compiled-language plugins loaded at runtime with no recompile of the shell. A plugin is an ordinary `cdylib` the shell `dlopen`s via `zmodload -R <path>`. The host/plugin boundary is `#[repr(C)]` structs + `extern "C"` function pointers; nothing about Rust's unstable layout, allocator, or panic ABI crosses it. The matching package manager is the [`znative`](https://github.com/MenkeTechnologies/zshrs/blob/main/docs/ZNATIVE.md) builtin (`znative load owner/repo`).

### [`zshrs`](https://github.com/MenkeTechnologies/zshrs) &middot; [`docs`](https://github.com/MenkeTechnologies/zshrs/blob/main/docs/PLUGINS.md) &middot; [`crates.io`](https://crates.io/crates/znative)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Install](#0x01-install)
- [\[0x02\] Quick start](#0x02-quick-start)
- [\[0x03\] Host API](#0x03-host-api)
- [\[0x04\] Loading & managing](#0x04-loading--managing)
- [\[0x05\] ABI versioning](#0x05-abi-versioning)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

Shells have two extension models: interpreted script plugins, and native modules bound to the shell's private internal headers (bash `enable -f`, zsh `zmodload`) — welded to one build, no stable ABI. `znative` is a third: a **published, versioned ABI package**. A third party runs `cargo add znative`, writes a handler, ships a `cdylib`, and it loads into any compatible zshrs — native speed, no shell source tree, no recompile, and version-mismatched plugins refused at load rather than crashing.

---

## [0x01] INSTALL

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
znative = "0.12"
```

---

## [0x02] QUICK START

```rust
use znative::{declare_plugin, Args, Host};
use std::os::raw::c_int;

fn rhello(host: &Host, args: &Args) -> c_int {
    host.print(&format!("hello, {}\n", args.rest().join(" ")));
    0
}

declare_plugin! {
    name: "hello",
    version: "0.1.0",
    builtins: { "rhello" => rhello },
}
```

```console
$ cargo build
$ zshrs
% zmodload -R target/debug/libhello.dylib   # .so on Linux
% rhello world
hello, world
```

---

## [0x03] HOST API

Inside a handler, `Host` is the shell's callback table:

| Method | Purpose |
| --- | --- |
| `host.print(s)` | write to the shell's stdout |
| `host.eval(code) -> i32` | run shell code, return its exit status |
| `host.getvar(name) -> Option<String>` | read a shell scalar parameter |
| `host.setvar(name, value)` | set a shell scalar parameter |
| `host.getfunction(name) -> Option<String>` | read a shell function's deparsed body (`${functions[name]}`) |
| `host.addfunction(name, body) -> bool` | define/replace a shell function (`functions[name]=body`) |
| `host.register_builtin(name, fn)` | register a command handler dynamically |
| `host.add_match(word)` | emit one completion candidate |
| `host.install_completion(cmd, gen)` | wire a native (Rust) completion into compsys |

`Args` gives `.name()` (`argv[0]`), `.rest()` (the arguments), and `.to_vec()` (all of argv). A handler returns the command's exit status (`0` = success), like a shell builtin.

`getfunction`/`addfunction` are the only structured access to shell *functions*. Because `addfunction` then `getfunction` round-trips a body through the shell's own parser and pretty-printer, the pair doubles as **deparse-as-a-service** — define a temp function from arbitrary source, read back its normalized form.

A plugin can also ship **native (Rust) completions**: add a `completions:` block to `declare_plugin!` mapping a command to a generator that emits candidates with `host.add_match`.

---

## [0x04] LOADING & MANAGING

Low level (`zmodload -R`):

```console
zmodload -R <path>...    # load
zmodload -R              # list loaded plugins
zmodload -uR <name>...   # unload by name
```

For distribution, the [`znative`](https://github.com/MenkeTechnologies/zshrs/blob/main/docs/ZNATIVE.md) package manager builds and loads straight from a repo — one line in `.zshrc`, self-installing on first start:

```console
znative load MenkeTechnologies/zshrs-forgit
```

---

## [0x05] ABI VERSIONING

`ABI_VERSION` is bumped on any layout/semantic change to the ABI structs. The host refuses to load a plugin whose `abi_version` does not match its own (a wrong struct layout would be undefined behaviour). Keep this crate's major/minor aligned with your target zshrs release.

---

## [0xFF] LICENSE

MIT. See [LICENSE](https://github.com/MenkeTechnologies/zshrs/blob/main/LICENSE).
