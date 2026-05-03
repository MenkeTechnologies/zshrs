# SHELL_IDS — Federated Recorder Identity Registry

`shell_id` is the per-row tag the daemon's canonical catalog uses to keep
records from different shells distinct in the same store. Any client —
`zshrs-recorder`, a bash wrapper, a fish function, an editor extension —
sets `shell_id` once per bundle/record so cross-shell queries
(`definitions_query`, `definitions_diff`) can filter and compare cleanly.

## When you need a `shell_id`

- **`recorder_ingest`** — bundle's top-level `shell_id` field stamps every
  row in the bundle. Defaults to `"zshrs"` when unset (preserves
  compatibility with pre-federation recorder builds).
- **`definitions_emit`** — REQUIRED. Single-record write API for shells
  that don't ship a full recorder (bash, ksh, dash, fish on demand).
- **`definitions_query`** — OPTIONAL `shell_id` filter. Omitting it
  returns rows from all shells. `"zshrs"` matches both explicit
  `"zshrs"` rows and pre-tagged legacy rows.
- **`definitions_diff`** — REQUIRED `shell_a` + `shell_b`.

Rows tagged with one shell_id never silently overwrite rows tagged with
a different shell_id — `(kind, name, shell_id)` is the de-facto primary
key for cross-shell queries even though the canonical engine still keys
by `(kind, name)` internally (latest write wins per name; the diff op
distinguishes by shell_id at query time).

## Reserved identifiers

These names are **reserved** — clients implementing those shells MUST
use exactly the listed identifier so the federated catalog stays
consistent across hosts:

| `shell_id`   | Shell                                  | Notes                                            |
|--------------|----------------------------------------|--------------------------------------------------|
| `zshrs`      | zshrs (this project)                   | Default for `recorder_ingest` bundles.           |
| `zsh`        | zsh (upstream, GNU/macOS system zsh)   | Use when running zshrs-recorder under `/bin/zsh`.|
| `bash`       | bash (3.x or 5.x)                      |                                                  |
| `dash`       | Debian Almquist Shell                  | Login/POSIX shells.                              |
| `ksh`        | ksh93 (or AT&T ksh derivatives)        |                                                  |
| `mksh`       | MirBSD Korn Shell                      |                                                  |
| `fish`       | Friendly Interactive Shell             |                                                  |
| `nu`         | Nushell                                | Not `nushell`.                                   |
| `elvish`     | Elvish                                 |                                                  |
| `pwsh`       | PowerShell Core (cross-platform)       | Not `powershell` or `posh`.                      |
| `xonsh`      | xonsh                                  |                                                  |
| `oil` / `ysh`| Oil Shell                              | `ysh` for the YSH-mode dialect; `oil` for OSH.   |
| `csh`        | C Shell                                |                                                  |
| `tcsh`       | TENEX C Shell                          |                                                  |

## Vendor-prefixed identifiers

Tools that act as recorder clients but aren't shells themselves should
use a `vendor:tool` form so they don't squat on a future shell name:

| `shell_id`            | Source                                 |
|-----------------------|----------------------------------------|
| `editor:vscode`       | VS Code extension publishing settings.  |
| `editor:helix`        | Helix integration.                     |
| `editor:nvim`         | Neovim plugin.                         |
| `ci:github-actions`   | CI runner injecting workflow context.  |
| `ci:gitlab`           | GitLab CI.                             |
| `agent:claude-code`   | Claude Code session-state hooks.       |
| `tool:asdf`           | asdf-vm shimmed env.                   |
| `tool:direnv`         | direnv-managed env vars.               |

## Identifier rules

- ASCII lowercase, digits, `-`, `_`, `:` only.
- Length 1–32 characters.
- No leading/trailing `-` `_` `:`.
- Reserved table is authoritative — pick a name from it before inventing
  one.
- For shells not yet on the table: open a PR adding the row before
  shipping a recorder client. The daemon does NOT validate against this
  list (it's a registry, not a schema), so collisions are caught by code
  review, not the runtime.

## Examples

### bash wrapper pushing a record

```bash
source examples/daemon-shell.zsh        # works under bash too
export DAEMON_SHELL_ID=bash
daemon-record-alias ll 'ls -al'
daemon-record-export EDITOR vim
daemon-record-bindkey '^R' history-incremental-search-backward
```

### Querying just bash rows

```bash
daemon-defs-query --kind alias --shell-id bash
```

### Diffing zsh against zshrs

```bash
daemon-defs-diff zsh zshrs alias
# → {"added":[...], "removed":[...], "changed":[...], "summary":{...}}
```

### Recorder bundle from fish

```fish
# inside fish's recorder client, the JSON sent to /op/recorder_ingest
# carries the federated identity at the top level:
{
  "shell_id": "fish",
  "events": [ ... ],
  ...
}
```

## Pre-federation rows

CanonicalRow shipped with `shell_id: None` before this registry existed.
The daemon treats `shell_id == None` as `"zshrs"` for filter/diff
purposes so legacy rows remain queryable without a forced rewrite. New
writes from `recorder_ingest` always populate the field explicitly.
