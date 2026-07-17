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
| `host.add_match(word)`          | emit one completion candidate (see below) |
| `host.install_completion(cmd, gen)` | wire a native completion into compsys  |

`Args` decodes `argv`: `.name()` is `argv[0]`, `.rest()` the arguments,
`.to_vec()` the whole vector.

A handler returns the command's exit status (`0` = success), exactly like
a shell builtin.

## Native completions

A plugin can provide a **completion written in Rust** for a command. Add a
`completions:` section to `declare_plugin!`, mapping the command to a
*generator* function. The generator receives `$CURRENT` (1-based index of
the word being completed) followed by every word on the line, and emits
candidates with `host.add_match`:

```rust
const NAMES: &[&str] = &["alice", "bob", "carol", "dave", "erin"];

fn greet_complete(host: &Host, args: &Args) -> c_int {
    let a = args.rest();                       // [CURRENT, word0, word1, ...]
    let current: usize = a.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let words = &a[1..];                        // words[0] == "greet"
    let prefix = current.checked_sub(1).and_then(|i| words.get(i))
        .map(String::as_str).unwrap_or("");
    for &c in NAMES {
        if c.starts_with(prefix) { host.add_match(c); }
    }
    0
}

declare_plugin! {
    name: "greet",
    version: "0.1.0",
    builtins:    { "greet" => greet },
    completions: { "greet" => greet_complete },
}
```

Then `greet <TAB>` offers `alice bob carol dave erin`, filtered by the
current prefix — all decided in Rust.

How it wires up: the macro registers the generator as a hidden builtin and
records a completion request. The host defers the `compdef` wiring until
the first time completion fires (a safe point in the completion pipeline;
doing it during plugin-init would run compsys glue too early). So **load
the plugin after `compinit`**. No leading-underscore is used for the
generator name — zsh treats `_*` command names as autoloadable completers,
which would shadow the builtin.

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

## Installing with zpm

`zmodload -R` is the low-level primitive. For distribution, **zpm** (zshrs's
package manager) installs a plugin straight from a GitHub repo — it clones,
`cargo build --release`s the cdylib, and `zmodload -R`s it:

```bash
zpm add MenkeTechnologies/zshrs-forgit       # native (Rust) plugin
zpm add MenkeTechnologies/zshrs-git-fuzzy
zpm list                                     # installed plugins
zpm load                                     # (in .zshrc) load all at startup
zpm remove forgit
```

zpm auto-detects a native plugin from a `Cargo.toml` with a `cdylib`
crate-type (ordinary `*.plugin.zsh` script repos install too, no metadata
needed). An optional `zpm.toml` at the repo root supplies metadata and the
lib stem:

```toml
[plugin]
name = "forgit"
version = "0.1.0"
description = "forgit ported to a native zshrs plugin"

[native]
lib = "forgit"        # produces libforgit.{dylib,so}
```

A plugin published this way depends on the SDK as a git dependency so it
builds standalone: `zshrs-plugin = { git = "https://github.com/MenkeTechnologies/zshrs" }`.

### Script (`.zsh`) plugins

zpm installs ordinary zsh script plugins too — the ones that register ZLE
widgets, functions, and completions. These stay script (the native ABI
registers builtins and completions, not ZLE widgets), and zpm loads them
by adding the repo to `$fpath` and sourcing its `*.plugin.zsh`:

```bash
zpm add zdharma-continuum/history-search-multi-word   # Ctrl-R multi-word history search
```

`history-search-multi-word` is a good stress test: a ZLE widget with its
own forked syntax-highlighter, paged `POSTDISPLAY` output, live
`region_highlight`, and in-widget key reads. It installs, `zpm load`s, binds
`^R`, and runs under zshrs's ZLE unchanged.

`zpm add` also takes `github:owner/repo`, a `git+URL` (optionally with an
`@ref` tag/branch), and `path:DIR` for a local checkout (no network). A
runnable `.zshrc` using zpm is at [`examples/zshrc`](../examples/zshrc) —
the one line a startup file needs is `zpm load`, which is zero-network
(store + index only) and safe even before anything is installed.

## ABI versioning

`ABI_VERSION` in the `zshrs-plugin` crate is bumped on any change to the
`HostApi` / `PluginInfo` / `BuiltinFn` layout or semantics. The host
refuses to load a mismatched plugin. Keep the crate's major/minor aligned
with your target zshrs release. The single exported symbol every plugin
must provide is `zshrs_plugin_init` (generated by `declare_plugin!`).

## Examples

Two runnable examples ship in `examples/`.

[`plugin-hello/`](../examples/plugin-hello/) — builtins, host API:

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

[`plugin-complete/`](../examples/plugin-complete/) — a native Rust
completion for a `greet` command:

```bash
cargo build --manifest-path examples/plugin-complete/Cargo.toml
zshrs   # interactive
# % autoload -Uz compinit; compinit
# % zmodload -R examples/plugin-complete/target/debug/libgreet.dylib
# % greet <TAB>          → alice  bob  carol  dave  erin
# % greet --lang <TAB>   → rust  ruby  python  perl  go
```

[`plugin-kubectl/`](../examples/plugin-kubectl/) — a real-world completion:
`kubectl` completion that delegates to cobra's `kubectl __complete`, so it
tracks the installed kubectl version (subcommands, flags, live resources)
with no static tree to maintain. Published as:
`zpm add MenkeTechnologies/zshrs-kubectl-completion`.

[`plugin-forgit/`](../examples/plugin-forgit/) — the **forgit** git+fzf
plugin ported command-for-command from zsh (`ga glo gd gcf …`). See
[PORTING_ZSH_PLUGIN.md](PORTING_ZSH_PLUGIN.md) for the full zsh→Rust
walkthrough. Published as a standalone repo:
`zpm add MenkeTechnologies/zshrs-forgit`.

[`plugin-git-fuzzy/`](../examples/plugin-git-fuzzy/) — **git-fuzzy**'s
`status` command: a *self-reentrant* fzf UI (every preview/keybind
re-invokes a helper, plus a `--listen` live-reload watcher). Shows the shim
technique that lets fzf binds reach native builtins — see the "self-reentrant
fzf tools" section of the porting guide. Published as:
`zpm add MenkeTechnologies/zshrs-git-fuzzy`.

[`plugin-zsh-z/`](../examples/plugin-zsh-z/) — **zsh-z**, the frecency
directory jumper (`z <partial>`), reimplemented in Rust: the `~/.z` datafile,
the frecency formula, aging, matching, and all `z` options. `cd` is delegated
to the shell (`host.eval`) so `$PWD`/hooks stay correct; a `chpwd` hook does
the recording. Published as: `zpm add MenkeTechnologies/zshrs-zsh-z`.

## Porting an existing zsh plugin

If you have a zsh plugin (shell functions + aliases + completions) and want
it as a native plugin, [**PORTING_ZSH_PLUGIN.md**](PORTING_ZSH_PLUGIN.md)
is a step-by-step guide: the construct-by-construct mapping, the
list→fzf→act pattern, subprocess/tty handling, and what stays shell —
worked end-to-end on forgit.

## Safety notes

- The loaded `libloading::Library` is kept alive for the process (or until
  unload); its `Drop` is a `dlclose`.
- A plugin panicking across the C-ABI boundary is undefined behaviour.
  Handlers should catch their own panics or avoid panicking; the host does
  not unwind through the FFI boundary for you.
- Plugin builtins run synchronously in-process, even when backgrounded
  (`foo &`) — zshrs is non-forking and an in-process builtin has nothing
  to background.
