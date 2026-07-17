# Native (Rust) Plugins

zshrs hosts plugins written in a native compiled language (Rust), loaded
at runtime with no recompile of the shell. A plugin is an ordinary
`cdylib` the shell `dlopen`s through a stable, versioned C ABI. This is
unique to zshrs — see the comparison below.

## Rust plugins vs zsh plugins

zsh has two ways to extend it, and zshrs adds a third. All three still
work in zshrs (it runs zsh script plugins and its ported modules as
before); the Rust plugin path is **additive**.

|                     | zsh script plugin        | zsh native module (`zmodload`) | **zshrs Rust plugin** |
| ------------------- | ------------------------ | ------------------------------ | --------------------- |
| Language            | zsh script (interpreted) | C                              | Rust (any native lang via the C ABI) |
| Artifact            | `.zsh` text file         | `.so` built in-tree            | `.dylib` / `.so` `cdylib` |
| Build against       | nothing — it's sourced   | zsh's **private** internal headers, via the zsh build system (`.mdd` + `Src/Modules/`) | the published `zshrs-plugin` crate — `cargo add zshrs-plugin` |
| ABI stability       | n/a                      | **none** — bound to the exact zsh build; no `MODULE_ABI_VERSION` guard exists | stable, versioned `ABI_VERSION`; mismatches refused at load |
| Load mechanism      | `source` → parse + interpret every startup | `dlopen` + `dlsym` of `NAME_setup_`/`_boot_`/`_features_` | `dlopen` + `dlsym` of one symbol, `zshrs_plugin_init` |
| Execution           | interpreted each call     | native machine code            | native machine code   |
| Startup cost        | re-parsed on every shell start (the cost `zinit turbo`/`zcompile` fight) | `dlopen` once            | `dlopen` once         |
| Registers           | functions, aliases, options, ZLE widgets, fpath completions | builtins, params, hooks (internal API) | builtins + host callbacks (`print`/`eval`/`getvar`/`setvar`) |
| Third-party viable  | yes (the whole ecosystem — oh-my-zsh, zinit) | **no in practice** — needs the zsh source tree and tracks its internals, so ~only zsh-bundled modules exist | **yes** — depend on one crates.io crate, no zshrs source needed |
| Type safety         | none                     | C (unchecked against zsh internals) | Rust type system, checked against the ABI crate |
| Failure mode        | shell error, recoverable  | crash/UB against internal symbols | UB only if a handler panics across the FFI boundary (see Safety notes) |

The distinction that matters: zsh already loads compiled `.so` modules,
but only through its **internal** C API — you build against zsh's private
headers, in its build tree, with no stable ABI, so a module is welded to
one zsh build. That is why the zsh plugin ecosystem is almost entirely
interpreted scripts and the native modules are almost entirely the ones
zsh ships itself. zshrs instead exposes a **stable, published, versioned
C ABI** (the `zshrs-plugin` crate): a third party runs `cargo add
zshrs-plugin`, writes a handler, ships a `cdylib`, and it loads into any
compatible zshrs — native speed, no shell source tree, no recompile of
the shell. First compiled Unix shell to offer that.

## Architecture

```
┌────────────────────────┐        ┌──────────────────────────┐
│ zshrs (host)           │        │ libfoo.dylib (plugin)    │
│                        │        │                          │
│ zmodload -R libfoo ────┼─dlopen─▶ zshrs_plugin_init(host)  │
│                        │        │   host.register_builtin  │
│ plugin_host registry   ◀────────┤   ("foo", handler)       │
│                        │        │                          │
│ execute_external_bg    │        │                          │
│   └ plugin_host::      │        │                          │
│      dispatch("foo") ──┼─call───▶ handler(host,argc,argv)  │
└────────────────────────┘        └──────────────────────────┘
        stable C ABI: zshrs-plugin crate (#[repr(C)])
```

- **Host loader**: `src/extensions/plugin_host.rs` — `dlopen` via
  `libloading`, an in-process command registry, and the host-callback
  table plugins call back through.
- **Shared ABI**: the [`zshrs-plugin`](../plugin-sdk/) crate. Both the
  host and every plugin depend on it, so both agree on the exact
  `#[repr(C)]` struct layout. `ABI_VERSION` gates loading: a plugin whose
  version does not match the host is refused (a wrong layout would be
  undefined behaviour).
- **Command resolution**: fusevm compiles names it does not recognise as
  builtins into *external* execution. A plugin command is unknown at
  compile time, so it arrives at `execute_external_bg`
  (`src/vm_helper.rs`), which consults `plugin_host::dispatch` **before**
  spawning a process — the analog of zsh's `resolvebuiltin` slot for
  `zmodload -ab` autoloaded builtins. Plugin commands therefore resolve
  after real builtins and shell functions, before PATH lookup.

## Writing a plugin

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
    let who = if args.rest().is_empty() {
        "world".into()
    } else {
        args.rest().join(", ")
    };
    host.print(&format!("hello, {who}\n"));
    0
}

declare_plugin! {
    name: "hello",
    version: "0.1.0",
    builtins: {
        "rhello" => rhello,
    },
}
```

`cargo build` produces `libhello.dylib` (macOS) or `libhello.so` (Linux).

## Host API

Inside a handler, `Host` is the shell's callback table:

| Method                          | Purpose                                   |
| ------------------------------- | ----------------------------------------- |
| `host.print(s)`                 | write to the shell's stdout               |
| `host.eval(code) -> i32`        | run shell code, return its exit status    |
| `host.getvar(name) -> Option`   | read a shell scalar parameter             |
| `host.setvar(name, value)`      | set a shell scalar parameter              |
| `host.register_builtin(n, f)`   | register a command handler dynamically    |

`Args` decodes `argv`: `.name()` is `argv[0]`, `.rest()` the arguments,
`.to_vec()` the whole vector.

A handler returns the command's exit status (`0` = success), exactly like
a shell builtin.

## Managing plugins

```bash
zmodload -R <path>...    # load each cdylib
zmodload -R              # list loaded plugins:  name  version  path
zmodload -uR <name>...   # unload each plugin by name
```

`-R` without `-A` is a zshrs extension. In C zsh `-R` removes a module
alias and is only meaningful alongside `-A`; `zmodload -A -R <name>` keeps
that behaviour, so no zsh parity is lost.

Loading a plugin whose name is already loaded is refused — unload first.
Unload purges the plugin's command registrations **before** `dlclose`, so
no live function pointer survives the library it lives in.

## ABI versioning

`ABI_VERSION` in the `zshrs-plugin` crate is bumped on any change to the
`HostApi` / `PluginInfo` / `BuiltinFn` layout or semantics. The host
refuses to load a mismatched plugin. Keep the crate's major/minor aligned
with your target zshrs release. The single exported symbol every plugin
must provide is `zshrs_plugin_init` (generated by `declare_plugin!`).

## Example

A runnable example is in [`examples/plugin-hello/`](../examples/plugin-hello/):

```bash
cargo build --manifest-path examples/plugin-hello/Cargo.toml
zshrs -c '
  zmodload -R examples/plugin-hello/target/debug/libhello.dylib
  rhello alice bob
  renv HOME
  renv MYVAR hi; echo $MYVAR
  zmodload -uR hello
'
```

## Safety notes

- The loaded `libloading::Library` is kept alive for the process (or until
  unload); its `Drop` is a `dlclose`.
- A plugin panicking across the C-ABI boundary is undefined behaviour.
  Handlers should catch their own panics or avoid panicking; the host does
  not unwind through the FFI boundary for you.
- Plugin builtins run synchronously in-process, even when backgrounded
  (`foo &`) — zshrs is non-forking and an in-process builtin has nothing
  to background.
