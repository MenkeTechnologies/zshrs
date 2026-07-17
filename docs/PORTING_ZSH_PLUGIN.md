# Porting a zsh plugin to a native Rust plugin

This guide walks through converting an existing zsh plugin into a native
zshrs plugin (a compiled `cdylib` loaded with `zmodload -R`). The worked
example is [**forgit**](../examples/plugin-forgit/) — a git+fzf plugin —
ported command-for-command.

Read [PLUGINS.md](PLUGINS.md) first for the plugin/host model and the
`declare_plugin!` API. This document is about the *translation*.

## The mental model

A zsh plugin is a set of shell **functions**, usually exposed through
**aliases** and **completions**. The native equivalent:

| zsh plugin construct | Rust plugin equivalent |
| --- | --- |
| a shell function `foo() { … }` | a builtin `fn foo(&Host, &Args) -> c_int` |
| `alias g='foo'` (the user-facing name) | register the builtin under the alias name directly (`"g" => foo`) |
| `compdef _foo foo` + `_foo()` completer | a `completions: { "g" => gen }` entry (see PLUGINS.md) |
| `$1 $2 "$@"` positional args | `args.rest()` (`&[String]`) |
| `$VAR` / `$FZF_DEFAULT_OPTS` | `host.getvar("VAR")` |
| `echo` / `print` to the terminal | `host.print(...)`, or let a subprocess inherit stdout |
| calling `git`, `fzf`, `fd`, `rg`, … | `std::process::Command` — it's native Rust, spawn anything |
| running shell code you don't want to port | `host.eval("…")` |
| `sed`/`awk`/`grep` text munging | do it in Rust (`str` methods, iterators) |

The key realization: **a Rust plugin is native code**, so it does not need
a host API to run external programs, read files, or parse text — it uses
the standard library directly. The host API is only for *shell
integration* (registering commands/completions, reading shell parameters,
printing to the shell, evaluating shell code).

## Why port

- **No per-invocation autoload/parse.** A zsh plugin function is parsed
  (or `.zwc`-loaded) and interpreted every call. The builtin is machine
  code, resolved once at `zmodload -R`.
- **Control flow is type-checked**, not re-parsed shell. The fragile parts
  of shell plugins — quoting, word-splitting, `$IFS`, `[[ ]]` vs `[ ]` —
  move into Rust where the compiler checks them.
- **The genuinely-shell parts stay shell.** fzf `--preview` strings run
  per-item via `sh -c`; you keep those as strings. You are not forced to
  reimplement a pager or fzf.

## The porting recipe

1. **Inventory the commands.** List every user-facing alias and the
   function it maps to. forgit's `forgit.plugin.sh` aliases `ga grh glo gd
   gi gcf gclean gss gcp` → nine functions. Each becomes one builtin,
   registered under the **alias** name.
2. **Find the shared helpers and config.** forgit resolves pagers and a
   base `FORGIT_FZF_DEFAULT_OPTS` block at load, and every command opens
   with `forgit::inside_work_tree`. Port these to free functions / a small
   context struct built per call.
3. **Port each command's skeleton:** guard → build a list → pick with fzf →
   act. Keep git/fzf as subprocesses; do the list transforms in Rust.
4. **Decide what stays shell.** fzf previews and `enter:execute(...)`
   binds are shell strings — keep them (interpolating resolved values like
   the diff pager). Everything else becomes Rust.
5. **Register with `declare_plugin!`** and build as a `cdylib`.

## The patterns, with forgit before/after

### Guard: `inside_work_tree`

```sh
# zsh
forgit::inside_work_tree() { git rev-parse --is-inside-work-tree >/dev/null; }
forgit::add() { forgit::inside_work_tree || return 1; … }
```

```rust
// Rust
fn inside_work_tree() -> bool {
    Command::new("git").args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null()).stderr(Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}
fn ga(host: &Host, args: &Args) -> c_int {
    if !inside_work_tree() { return 1; }
    // …
}
```

### Reading shell/config values

```sh
forgit_pager=${FORGIT_PAGER:-$(git config core.pager || echo 'cat')}
opts="$FZF_DEFAULT_OPTS --ansi --height='80%' …"
```

```rust
let pager = host.getvar("FORGIT_PAGER")
    .filter(|s| !s.is_empty())
    .or_else(|| git_config("core.pager"))   // helper wrapping `git config`
    .unwrap_or_else(|| "cat".into());
let base = format!("{}\n--ansi\n--height=80%\n…", host.getvar("FZF_DEFAULT_OPTS").unwrap_or_default());
```

### List → fzf → selection

The heart of forgit. In shell it's a pipeline; in Rust it's: capture the
git list, spawn fzf, write the list to its stdin, read the selection.

```sh
files=$(git ls-files --modified "$(git rev-parse --show-toplevel)" \
    | FZF_DEFAULT_OPTS="$opts" fzf --preview="$cmd")
[[ -n "$files" ]] && echo "$files" | tr '\n' '\0' | xargs -0 -I% git checkout %
```

```rust
let list = git_capture(&["ls-files", "--modified", &toplevel()]);
let preview = format!("git diff --color=always -- {{}} | {}", diff_pager);
if let Some(sel) = fzf(&list, &opts, Some(&preview)) {
    for f in sel.lines().filter(|l| !l.is_empty()) {
        git_run(&["checkout", f]);   // act on each selected file
    }
}
```

The `fzf` helper is reusable across every command:

```rust
fn fzf(input: &str, opts_env: &str, preview: Option<&str>) -> Option<String> {
    let mut cmd = Command::new("fzf");
    cmd.env("FZF_DEFAULT_OPTS", opts_env);
    if let Some(p) = preview { cmd.arg(format!("--preview={p}")); }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped());   // stderr inherited: fzf drives the tty
    let mut child = cmd.spawn().ok()?;
    // Write the list on a thread so a large list can't deadlock against
    // fzf reading stdin while we wait on its stdout.
    let mut stdin = child.stdin.take()?;
    let owned = input.to_string();
    let writer = std::thread::spawn(move || { let _ = stdin.write_all(owned.as_bytes()); });
    let out = child.wait_with_output().ok()?;
    let _ = writer.join();
    if !out.status.success() { return None; }           // aborted / no match
    let sel = String::from_utf8_lossy(&out.stdout);
    let sel = sel.trim_end_matches('\n');
    (!sel.is_empty()).then(|| sel.to_string())
}
```

Three things the guide-reader must get right here:

- **stderr is inherited**, stdin/stdout are pipes. fzf reads its candidate
  list from the stdin pipe and opens `/dev/tty` itself for the interactive
  UI — so it works even though stdin isn't a terminal.
- **Write stdin on a thread.** Writing the whole list then calling
  `wait_with_output()` can deadlock for large lists (fzf's stdin pipe
  fills before it drains it while you block reading stdout). The writer
  thread avoids it.
- **fzf exits non-zero on abort/no-match** — treat that as "no selection",
  not an error.

### Text munging: port sed/grep to Rust

forgit's `ga` greps colored `git status` output and rewrites it with
`sed`. The port uses stable porcelain and parses it in Rust — more robust
than color-grepping:

```sh
files=$(git -c color.status=always status -su | grep -F -e "$changed" … | sed -E 's/…/[\1]  \2/')
```

```rust
let porcelain = git_capture(&["status", "--porcelain", "-u"]);
for line in porcelain.lines() {
    let (xy, path) = line.split_at(2);
    let path = path.trim_start();
    let wt = xy.as_bytes().get(1).copied().unwrap_or(b' ');
    if xy == "??" || wt != b' ' || xy.starts_with('U') {   // unstaged/untracked/unmerged
        list.push_str(path); list.push('\n');
    }
}
```

### What stays shell: fzf previews

fzf runs `--preview` (and `enter:execute(...)`) via `sh -c` per item, so
those are irreducibly shell. Keep them as strings, interpolating the
values you resolved in Rust:

```rust
let preview = format!(
    "echo {{}} | cut -d: -f1 | xargs -I% git stash show --color=always --ext-diff % | {}",
    diff_pager);
let opts = format!("{base}\n+s +m -0 --tiebreak=index\n\
                    --bind=enter:execute({preview} | LESS=-R less)");
```

This is not a cop-out — the original plugin *also* shipped these as shell
strings. The port moves the *control flow* to Rust and leaves the
per-item shell snippets where they belong.

## Registering

```rust
declare_plugin! {
    name: "forgit",
    version: "0.1.0",
    builtins: {
        "ga"     => ga,      // was: alias ga='forgit::add'
        "grh"    => grh,
        "glo"    => glo,
        "gd"     => gd,
        "gcf"    => gcf,
        "gclean" => gclean,
        "gss"    => gss,
        "gcp"    => gcp,
        "gi"     => gi,
    },
}
```

```bash
cargo build --manifest-path examples/plugin-forgit/Cargo.toml
zshrs
# % zmodload -R examples/plugin-forgit/target/debug/libforgit.dylib
# % ga            # interactive git add, native
```

## Checklist / gotchas

- **Register under the alias name**, not the internal function name — the
  user types `ga`, not `forgit::add`.
- **Runtime deps are the same.** forgit needs `git` and `fzf` on `PATH`;
  so does the port. `Command::new` fails cleanly if they're missing.
- **Inherit the tty for interactive children** (fzf, `less`, editors):
  don't capture their stderr.
- **Don't over-port.** Reimplementing `git`, a pager, or fzf in Rust is
  the wrong move — shell out to them. Port the glue, not the tools.
- **Completions** (if the plugin ships `_foo` completers) map to the
  `completions:` block — see [PLUGINS.md](PLUGINS.md).
- **No leading underscore** on any builtin name — zsh treats `_*` command
  names as autoloadable completers.

The full, buildable result is in
[`examples/plugin-forgit/src/lib.rs`](../examples/plugin-forgit/src/lib.rs).

## Advanced: self-reentrant fzf tools (git-fuzzy)

forgit is "list → fzf → act": fzf runs once. Tools like
[**git-fuzzy**](../examples/plugin-git-fuzzy/) are **self-reentrant** — the
fzf UI calls *back* into the tool on every keystroke: `--preview 'git fuzzy
helper status_preview {…}'`, `--bind '<key>:execute(git fuzzy helper
status_add {+2..})+reload(…)'`, a `--listen` port that a background watcher
POSTs live reloads to. In bash this re-execs the `git-fuzzy` script and
re-sources its library **per keystroke** — git-fuzzy even has "dispatch-aware
sourcing" to keep that cheap. That per-keystroke sourcing is exactly the
overhead a compiled host exists to delete.

The wrinkle: fzf runs its bind/preview commands via `sh`, which **cannot
call a plugin builtin**. Three parts make it work:

**1. Helpers are builtins too.** The plugin registers `gf` and dispatches an
internal `--helper <sub>` mode: `gf --helper status_preview …`,
`gf --helper status_add …`. Same code path, no separate binary.

**2. A shim bridges fzf → the plugin.** Since `sh` can't reach the builtin,
generate a tiny shim once and point every fzf bind at it:

```rust
// ~/.cache/zshrs/gf-helper.sh
//   #!/bin/sh
//   exec <zshrs> -fc 'zmodload -R "$0" 2>/dev/null; gf --helper "$@"' <self.dylib> "$@"
```

fzf's binds become `execute(<shim> status_add {+2..})`. Each invocation is a
fresh `zshrs` that `dlopen`s this plugin (an mmap'd dylib — no parsing) and
runs the helper builtin. That replaces bash's per-keystroke library sourcing
with a single dlopen. The plugin finds **its own** dylib path with `dladdr`
and the zshrs binary with `std::env::current_exe()`; pass the dylib as `$0`
(not embedded in the `-c` script) so a path with spaces survives.

**3. Live reload without `curl`.** git-fuzzy's watcher POSTs `reload-sync(…)`
to fzf's `--listen` port. Port it to a raw `TcpStream` write — no `curl`
dependency, and the watch loop itself is Rust:

```rust
fn fzf_post(port: u16, action: &str) -> bool {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return false };
    let req = format!("POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\
                       Connection: close\r\n\r\n{}", action.len(), action);
    s.write_all(req.as_bytes()).is_ok()   // connect failing == fzf gone -> stop watching
}
```

**Verifying interactive TUIs.** The *logic* (helpers, the shim round-trip,
menu generation, the `--expect` interpreter, geometry, diff rendering) is
unit-testable headlessly: call `gf --helper …` directly, drive the top-level
command with a stub `fzf` that echoes a selection, and invoke the generated
shim by hand. The **live fzf render** — the actual full-screen UI reacting to
keys — must be checked in a **real terminal**; a scripted PTY can drive a
plain `sh | fzf` but does not faithfully reproduce an interactive shell
handing the terminal to a full-screen child, so don't treat a blank PTY
capture as a failure. Verify the UI by running it.

The full port of git-fuzzy's `status` command (preview, inspect, stage /
unstage / discard / amend / patch / commit / edit, and the `--listen`
live-reload watcher) is in
[`examples/plugin-git-fuzzy/src/lib.rs`](../examples/plugin-git-fuzzy/src/lib.rs).
