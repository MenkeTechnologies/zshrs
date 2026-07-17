# Native (Rust) Plugins

zshrs hosts plugins written in a native compiled language (Rust), loaded
at runtime with no recompile of the shell. A plugin is an ordinary
`cdylib` the shell `dlopen`s through a stable, versioned C ABI. This is
unique to zshrs — see the comparison below.

## Rust plugins vs zsh plugins

Shells already have two extension models, and zshrs adds a third. All
still work in zshrs (it runs zsh script plugins and its ported modules as
before); the Rust plugin path is **additive**.

Native runtime plugin loading itself is **not** new — bash has
`enable -f file.so name` (loadable builtins, since ~1996) and zsh has
`zmodload` for C modules. What is new is *how* zshrs exposes it:

|                     | script plugin (`.zsh`)   | bash `enable -f` / zsh `zmodload` native builtin | **zshrs Rust plugin** |
| ------------------- | ------------------------ | ------------------------------ | --------------------- |
| Language            | shell script (interpreted) | C                            | Rust (any native lang via the C ABI) |
| Artifact            | `.zsh` text file         | `.so` built in the shell's tree | `.dylib` / `.so` `cdylib` |
| Build against       | nothing — it's sourced   | the shell's **private** internal headers (`builtins.h`/`shell.h` for bash; the `.mdd` + `Src/Modules/` build for zsh) | the published `zshrs-plugin` crate — `cargo add zshrs-plugin` |
| ABI stability       | n/a                      | **none** — bound to the exact shell build; no version magic in either | stable, versioned `ABI_VERSION`; mismatches refused at load |
| Distribution        | a file to source         | must track and rebuild against each shell release | one crates.io SDK crate, independent of the shell's source |
| Load / unload       | `source` (re-parse every startup) | `enable -f` / `-d` (bash); `zmodload` (zsh) — `dlsym` internal symbols | `zmodload -R` / `-uR` — `dlsym` one symbol, `zshrs_plugin_init` |
| Execution           | interpreted each call     | native machine code            | native machine code   |
| Registers           | functions, aliases, options, ZLE widgets, fpath completions | builtins/params/hooks via the shell's internal API | builtins + a curated host API (`print`/`eval`/`getvar`/`setvar`) |
| Third-party viable  | yes (oh-my-zsh, zinit)   | **rare in practice** — needs the shell source tree and tracks its internals, so ~only the shell's own bundled modules exist | **yes** — depend on one crates.io crate, no zshrs source needed |
| Type safety         | none                     | C (unchecked against shell internals) | Rust type system, checked against the ABI crate |

The distinction that matters is **not** "loads native code" — bash and zsh
both do. It is that bash's and zsh's native interfaces are their
**internal** C APIs: you compile against the shell's private headers, in
its build tree, with no stable ABI and no version gate, so a plugin is
welded to one shell build and can crash a mismatched one. That is why
neither ecosystem has meaningful third-party native plugins — the native
modules that exist are almost all the ones the shell ships itself.

zshrs instead exposes a **stable, published, versioned C ABI** — the
`zshrs-plugin` crate on crates.io, gated by `ABI_VERSION`: a third party
runs `cargo add zshrs-plugin`, writes a handler, ships a `cdylib`, and it
loads into any compatible zshrs — native speed, no shell source tree, no
recompile, and version-mismatched plugins refused rather than crashing.
First shell to make its native-plugin interface an independently-published,
versioned ABI package instead of its own build-tree internals.

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
