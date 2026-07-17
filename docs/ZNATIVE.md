# znative — the zshrs plugin package manager

`znative` is a built-in command for installing shell plugins. It handles both
**zsh script plugins** (the oh-my-zsh / zinit kind — functions, aliases, ZLE
widgets, completions) and **native Rust plugins** (`cdylib`s loaded through
the [`zshrs-plugin`](../plugin-sdk/) ABI — see [PLUGINS.md](PLUGINS.md)).

It is **global only**: one content-addressed store under `$ZSHRS_HOME/pkg/`,
no per-project manifest or lockfile. The whole workflow is one line per
plugin in your `.zshrc`:

```sh
znative load owner/repo
```

On the first shell start that installs the plugin and loads it; on every
start after, the same line loads it from the store with no network. There is
no separate install step. `znative` needs `git` on `PATH` for remote sources, and
`cargo` for native plugins that ship as source.

## Commands

| Command (aliases)              | Arguments   | What it does |
| ------------------------------ | ----------- | ------------ |
| `load` (`source`)              | `[NAME_or_SOURCE…]` | The one you need. With no argument, load every installed plugin. Given an installed **name** or a **source** already in the store, load it — **zero network**. Given a **source** not yet in the store (`owner/repo`, `github:…`, `git+URL`, `path:…`), install it first, then load. This is what a `.zshrc` calls. |
| `add` (`install`, `i`)         | `SOURCE…`   | Resolve, install into the store, record in the index, and load. (`load` self-installs, so this is mostly for installing without a `.zshrc` line.) Multiple sources allowed. |
| `remove` (`rm`, `uninstall`)   | `NAME…`     | Unload (native), delete the store copy, drop the index row. |
| `list` (`ls`)                  | —           | One line per installed plugin: `name  version  kind  source`. |
| `info` (`show`)                | `NAME`      | Full record: name, version, kind, source, store path, integrity, lib / files / fpath. |
| `update` (`upgrade`, `up`)     | `[NAME]`    | Re-resolve and reinstall from the recorded source (one, or all). |
| `help` (`-h`, `--help`)        | —           | Usage. |

Errors print as `znative: <reason>` on stderr and the command returns non-zero.

## Sources

The `add`/`update` spec is auto-classified:

| Form                              | Example                                   | Resolves to |
| --------------------------------- | ----------------------------------------- | ----------- |
| `owner/repo`                      | `zsh-users/zsh-autosuggestions`           | `git clone https://github.com/owner/repo` |
| `github:owner/repo`               | `github:zdharma-continuum/fast-syntax-highlighting` | GitHub clone (explicit) |
| `git+URL`                         | `git+https://gitlab.com/team/plug.git`    | `git clone URL` |
| a URL ending `.git` or with `://` | `https://example.com/x.git`               | `git clone URL` |
| `path:DIR`                        | `path:examples/plugin-revolver`           | local directory (no network) |
| an absolute / `./` / `../` / `~` path | `~/src/my-plugin`                     | local directory (no network) |

Any remote form may carry an `@ref` suffix (split after the last `/`) to pin a
tag, branch, or commit: `owner/repo@v1.2.0`, `git+https://host/x.git@main`.
Clones are shallow (`git clone --depth 1 [--branch REF]`); an arbitrary commit
sha that a shallow `--branch` clone can't reach falls back to a full clone +
`git checkout`.

## Plugin kinds

| Kind       | Loaded by                          | Built with |
| ---------- | ---------------------------------- | ---------- |
| **native** | `zmodload -R` (the plugin host)    | `cargo build --release` when no prebuilt `lib*.{dylib,so}` is present |
| **script** | `fpath=(DIR $fpath)` + `source *.plugin.zsh` | nothing — sourced as-is |

When there is no explicit `znative.toml`, the kind is auto-detected:

1. a prebuilt `lib*.{dylib,so}` at the repo root, **or** a `Cargo.toml`
   whose `[lib] crate-type` includes `cdylib` → **native**;
2. otherwise any `*.plugin.zsh`, a `functions/` directory, or `*.zsh`
   files → **script**;
3. otherwise `znative` reports it cannot determine the kind.

## The store

Everything lives under `$ZSHRS_HOME/pkg/` (default `~/.zshrs/pkg/`):

```
$ZSHRS_HOME/pkg/
  store/<name>@<version>/   # the installed plugin (content-addressed)
  installed.toml            # the global index — the source of truth
  git/                      # scratch: remote clones land here, then copy to store/
  cache/  bin/              # internal scratch
```

The copy into `store/` excludes `.git/` and `target/`, so the store holds only
loadable content. Each install is SHA-256 pinned as `sha256-<hex>` in
`installed.toml`. A record looks like:

```toml
[[package]]
name = "revolver"
version = "0.2.0"
source = "github:MenkeTechnologies/zshrs-revolver"
kind = "native"
integrity = "sha256-…"
lib = "librevolver.dylib"          # native: the cdylib to zmodload -R
# script plugins record instead:
# source_files = ["…​.plugin.zsh"]
# fpath = ["functions"]
```

## `znative.toml` (optional manifest)

A plugin repo may ship a `znative.toml` at its root to declare metadata and the
load recipe explicitly (it overrides auto-detection):

```toml
[plugin]
name = "git-fuzzy"
version = "0.1.0"
description = "git-fuzzy ported to a native zshrs plugin"

# Native (Rust cdylib) plugin — dlopened via `zmodload -R`:
[native]
lib = "git_fuzzy"        # produces lib<lib>.{dylib,so}
# build = true           # run `cargo build --release`; defaults to true
                         # when a Cargo.toml is present

# …or a script plugin:
# [script]
# source = ["git-fuzzy.plugin.zsh"]   # files to `source`, in order
# fpath  = ["functions"]              # dirs to prepend to $fpath
```

Standard oh-my-zsh / zinit `*.plugin.zsh` repos need no `znative.toml` at all.

## In your `.zshrc`

List the plugins you want with `znative load owner/repo`, one per line, in load
order. First start installs each; later starts load from the store with no
network:

```sh
znative load zdharma-continuum/history-search-multi-word
znative load zsh-users/zsh-autosuggestions
znative load MenkeTechnologies/zshrs-forgit
znative load zsh-users/zsh-syntax-highlighting   # keep highlighting last
```

A bare `znative load` (no argument) loads everything already in the store — handy
if you prefer to `znative add` interactively and keep just one line in `.zshrc`. A
complete example startup file is at [`examples/zshrc`](../examples/zshrc).

## Examples

```sh
# In .zshrc — self-installing on first use, zero-network after.
znative load zdharma-continuum/history-search-multi-word  # script: Ctrl-R multi-word search
znative load MenkeTechnologies/zshrs-forgit               # native: git+fzf
znative load MenkeTechnologies/zshrs-revolver             # native: progress spinner
znative load path:examples/plugin-revolver                # local checkout
znative load zsh-users/zsh-syntax-highlighting@0.8.0      # pinned ref
znative load git+https://gitlab.com/team/plugin.git       # non-GitHub URL

# Interactive store management.
znative add zsh-users/zsh-autosuggestions   # install without a .zshrc line
znative list                                # what's installed
znative info forgit                         # details for one
znative update                              # reinstall everything from source
znative remove forgit                       # unload + delete
```
