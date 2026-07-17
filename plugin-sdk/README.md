# zshrs-plugin

Stable C-ABI SDK for writing **native (Rust) plugins** for
[zshrs](https://github.com/MenkeTechnologies/zshrs) — the first compiled
Unix shell, and the first that hosts compiled-language plugins loaded at
runtime with no recompile of the shell.

A plugin is an ordinary `cdylib` the shell `dlopen`s via `zmodload -R
<path>`. The host/plugin boundary is a hand-rolled, versioned C ABI
(`#[repr(C)]` structs + `extern "C"` function pointers); nothing about
Rust's unstable layout, allocator, or panic ABI crosses it.

## Quick start

`Cargo.toml`:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
zshrs-plugin = "0.12"
```

`src/lib.rs`:

```rust
use zshrs_plugin::{declare_plugin, Args, Host};
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

## Host API

Inside a handler, `Host` exposes:

| Method | Purpose |
| --- | --- |
| `host.print(s)` | write to the shell's stdout |
| `host.eval(code)` | run shell code, get its exit status |
| `host.getvar(name)` | read a shell scalar (`Option<String>`) |
| `host.setvar(name, value)` | set a shell scalar |
| `host.register_builtin(name, fn)` | register a command dynamically |

`Args` gives `.name()` (`argv[0]`), `.rest()` (the arguments), and
`.to_vec()` (all of argv).

## Managing plugins

```console
zmodload -R <path>...    # load
zmodload -R              # list loaded plugins
zmodload -uR <name>...   # unload by name
```

## Versioning

`ABI_VERSION` is bumped on any layout/semantic change to the ABI structs.
The host refuses to load a plugin whose `abi_version` does not match its
own. Keep this crate's major/minor aligned with your target zshrs.
