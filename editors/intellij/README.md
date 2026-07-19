# zshrs JetBrains Plugin

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![IDE](https://img.shields.io/badge/IDE-2025.2%2B-orange.svg)](https://plugins.jetbrains.com/)
[![JDK](https://img.shields.io/badge/JDK-17-blue.svg)](https://adoptium.net/)
[![Plugin SDK](https://img.shields.io/badge/IntelliJ%20Platform%20Gradle-2.16-purple.svg)](https://plugins.jetbrains.com/docs/intellij/tools-intellij-platform-gradle-plugin.html)

### `[FULL IDE FRONT-END FOR THE FIRST COMPILED UNIX SHELL]`

> *"No fork, no problems — now with breakpoints."*

## `[BUILT FOR ZSHRS]`

A JetBrains-platform plugin that drives the LSP and DAP servers compiled into the `zshrs` binary. Hand-rolled lexer with **45 color slots**, semantic-token overlay from the LSP, **1388** hover-card-backed identifiers spanning every canonical builtin / keyword / option / special variable / compsys function / extension builtin, full breakpoint-debugger over DAP, a 7-tab reflection tool window that mirrors the runtime registries 1:1 (`all` / `builtins` / `keywords` / `options` / `special_vars` / `compsys` / `extensions`), Extract Variable / Constant / Function refactors plus Shift-F6 cross-file rename, run configs that auto-create from any `.zsh` / `.zshrc` / `.zshenv` / `.zlogin` / `.zlogout` / `.zprofile` / `.zpreztorc` file. Talks to the in-tree `src/extensions/lsp.rs` + `src/extensions/dap.rs` over JSON-RPC; no upstream `lsp-server` / `dap-types` crates anywhere in the build.

### [`zshrs`](https://github.com/MenkeTechnologies/zshrs) · [`Reference`](https://menketechnologies.github.io/zshrs/reference.html) · [`fusevm`](https://github.com/MenkeTechnologies/fusevm) · [`compsys`](../../src/compsys/) · [`strykelang`](https://github.com/MenkeTechnologies/strykelang)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Install](#0x01-install)
- [\[0x02\] Editor](#0x02-editor)
- [\[0x03\] LSP](#0x03-lsp)
- [\[0x04\] Code Actions](#0x04-code-actions)
- [\[0x05\] Reflection Tool Window](#0x05-reflection-tool-window)
- [\[0x06\] External Libraries](#0x06-external-libraries)
- [\[0x07\] Run / Debug](#0x07-run--debug)
- [\[0x08\] DAP Protocol](#0x08-dap-protocol)
- [\[0x09\] Refactor / Rename](#0x09-refactor--rename)
- [\[0x0A\] Configuration](#0x0a-configuration)
- [\[0x0B\] Logs](#0x0b-logs)
- [\[0x0C\] Building](#0x0c-building)
- [\[0x0D\] Plugin Architecture](#0x0d-plugin-architecture)
- [\[0x0E\] Version Compatibility](#0x0e-version-compatibility)
- [\[0x0F\] Limitations](#0x0f-limitations)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

zshrs ships an **LSP server** and **DAP debug adapter** built into the `zshrs` binary (`zshrs --lsp`, `zshrs --dap HOST:PORT`). This plugin is the JetBrains-side driver:

- Spawns the LSP / DAP servers on demand, frames JSON-RPC over stdio / TCP, and renders responses through the IDE's native UI affordances (gutter breakpoints, intentions popup, refactor menu, code-folding handles, semantic-tokens layer, reflection tool window).
- Adds **zero new shell-language code paths**. Everything the user sees in the editor comes from one of three sources: the hand-rolled `ZshrsLexer.kt` (instant first-paint highlighting), the `textDocument/semanticTokens` overlay (LSP-driven full classification), or the `--dump-reflection` JSON (tool-window inventory).
- No upstream `lsp-server` / `lsp-types` / `dap-types` / `lsp4ij` dependencies anywhere on the Rust side. JetBrains' own `LspServerSupportProvider` is the only LSP4J consumer; everything else is hand-framed JSON-RPC on top of `serde_json`. Same on the DAP side.

Compiled `editors/intellij/build/distributions/zshrs-intellij-<v>.zip` is **~1.7 MiB** and self-contained: only Kotlin stdlib + IntelliJ Platform classes at runtime.

---

## [0x01] INSTALL

```sh
# Install from disk: Settings → Plugins → ⚙ → Install Plugin from Disk…
# Then pick:
editors/intellij/build/distributions/zshrs-intellij-<version>.zip
```

After install: restart the IDE → open any `.zsh` / `.zshrc` / `.zshenv` / `.zlogin` / `.zlogout` / `.zprofile` / `.zpreztorc` file → the LSP starts automatically → the debugger activates the first time you click Debug.

The `zshrs` binary must be on `$PATH`, or configured under *Settings → Tools → zshrs → zshrs executable*. The plugin resolves the executable via `ZshrsSettings.zshrsExecutable` first, then falls back to `which zshrs`.

---

## [0x02] EDITOR

| Surface | Behavior |
|---------|----------|
| File association | `.zsh`, `.zshrc`, `.zshenv`, `.zlogin`, `.zlogout`, `.zprofile`, `.zpreztorc` (configurable; see [§0x0A](#0x0a-configuration)) |
| Lexer | Hand-rolled in `ZshrsLexer.kt` — instant first-paint highlighting before the LSP semantic-tokens response lands |
| Color slots | **45** stable `ZSHRS_*` `TextAttributesKey`s under *Settings → Editor → Color Scheme → zshrs* |
| Brace matching | `{` / `}`, `(` / `)`, `[` / `]` via `ZshrsBraceMatcher.kt` — also pairs `[[` / `]]` and `((` / `))` conditional/arithmetic forms |
| Comments | Cmd/Ctrl-`/` for `#` line comments via `ZshrsCommenter.kt` |
| Quote handler | `"`, `'`, `` ` ``, `$'...'` ANSI-C auto-pair; inside-string typing recognized via `ZshrsQuoteHandler.kt` |

### Lexer coverage

| Token category | Examples |
|----------------|----------|
| Comments | `#` line, `#!` shebang on line 1 |
| Strings | `"…"`, `'…'`, `` `…` ``, `$'…'` ANSI-C, `<<EOF` / `<<-EOF` / `<<'EOF'` heredocs |
| Numbers | `42`, `3.14`, `0xFF`, `1_000_000` |
| Reserved words | `if` / `then` / `else` / `elif` / `fi` / `for` / `foreach` / `while` / `until` / `do` / `done` / `case` / `esac` / `select` / `repeat` / `function` |
| Declaration | `local` / `typeset` / `declare` / `export` / `readonly` / `integer` / `float` / `private` |
| Modifier | `alias` / `setopt` / `zstyle` / `zmodload` / `autoload` / `bindkey` / `compdef` / `zcompile` |
| I/O | `source` / `.` / `eval` / `exec` / `echo` / `print` / `printf` / `read` / `trap` |
| Builtins | full canonical set from `ported::builtin::BUILTINS` (159 entries) |
| Variables | `$name`, `${name}`, `${(P)var}`, `$0…$9`, `$?`, `$!`, `$$`, `$#`, `$*`, `$@`, `$-`, `$_` |
| Operators | `|`, `|&`, `&&`, `||`, `&`, `=`, `+=`, `:=`, `?=`, `=~`, `==` |
| Redirects | `>`, `>>`, `<`, `<<`, `<<<`, `&>`, `2>&1`, `>&`, `>|` |
| Glob | `*`, `?`, `[…]`, `(#qX)` qualifier markers |
| Case terminators | `;;`, `;;&`, `;|` |
| Backslash escapes | `\<newline>` line continuation → whitespace; `\$`, `\"`, `\(` → escape (no longer flagged BAD_CHARACTER) |

---

## [0x03] LSP

The LSP server is in-process inside the `zshrs` binary — `zshrs --lsp` spawns it over stdio. Plugin side starts it via `ZshrsLspServerSupportProvider.kt`; descriptor in `ZshrsLspServerDescriptor.kt`.

### Capabilities

| Capability | Trigger / scope |
|------------|-----------------|
| `completion` | builtins, keywords, options, special vars, in-file functions; trigger chars `$` `{` `-` `:` plus all letters |
| `hover` | full markdown cards for **1388** identifiers (156 builtins, 31 keywords, 756 options, 279 specials, 52 compsys fns, 114 extension builtins) |
| `definition` / `references` | function names declared in the open document; cross-file for package-scoped symbols |
| `documentHighlight` | same scan as references |
| `documentSymbol` | `function foo`, `foo()`, `alias`, `local`/`typeset`/`export` decls |
| `foldingRange` | `{ … }`, `do … done`, `case … esac` blocks + ≥3 consecutive `#` comment runs |
| `rename` (with `prepareRename`) | scalars / arrays / hashes / function / alias names; cross-file for package symbols (see [§0x09](#0x09-refactor--rename)) |
| `semanticTokens/full` | token classes mirroring the lexer; LSP overlay refines what the hand lexer approximates |
| `codeAction` | Extract Variable / Constant / Function — see [§0x04](#0x04-code-actions) |
| `formatting` | trailing-whitespace strip, indent normalize, final-newline guarantee — Cmd/Ctrl-Opt-`L` |
| `publishDiagnostics` | brace + block matching, unclosed strings, mismatched `fi` / `done` / `esac`; lights up on `didOpen` / `didChange` / `didSave` |

### Doc cascade (`zshrs --docs NAME`)

`zshrs::lsp::lookup_doc(name)` walks five sources in order, first hit wins:

1. **Yodl-derived tables** (`src/extensions/zsh_*_docs.rs`) — generated once at build time from upstream `Doc/Zsh/*.yo` (`builtins.yo`, `options.yo`, `grammar.yo`, `params.yo`, plus every `mod_*.yo`, `compsys.yo`, `compwid.yo`, `contrib.yo`, `zftpsys.yo`, `calsys.yo`, `tcpsys.yo`). Authoritative for canonical zsh.
2. **`KEYWORD_DOCS`** hand fallback — sub-keywords (`then` / `else` / `do` / `esac` / `in` / `{` / `}`) that yodl documents only as part of compound statements.
3. **`BUILTIN_DOCS`** hand fallback — coreutils, `chdir` / `bye` / `declare` / `r` / `unfunction` / `zf_*` and other names with no per-name `item(tt(NAME))` block upstream.
4. **`SPECIAL_VAR_DOCS`** hand fallback — `$SHELL` / `$EDITOR` / `$VISUAL` + every well-known env var.
5. **`OPTION_DOCS_FALLBACK`** — `RESTRICTED` (the one option upstream doesn't document via an item block).
6. **`EXT_BUILTIN_DOCS`** — every entry in `ext_builtins::EXT_BUILTIN_NAMES` (91) + `daemon::builtins::ZSHRS_BUILTIN_NAMES` (23) = 114 hand bodies.
7. **`COMPSYS_FN_DOCS`** — fallback for compsys functions without a yodl item block (`_main_complete` / `_directories` / `_git` / `_docker` / `_cargo` / etc.).

Coverage is **gated by `tests/doc_coverage_audit.rs`** — 8 tests, every canonical name in every registry must resolve to a non-placeholder body. The gate also pins `keywords_inventory_matches_man_zshmisc_reserved_words` so the Keywords inventory tracks every reserved word in `man zshmisc` (including the declaration commands `local` / `typeset` / `export` / etc., which `man zshmisc` lists as reserved words and which also appear in the Builtins tab).

### Transport

- **Stdio**, Content-Length-framed JSON-RPC. Hand-rolled framer on top of `serde_json` — no `lsp-server` / `lsp-types` crates.
- Optional `ZSHRS_LSP_LOG=<path>` env var dumps every request/response to a file for debugging.
- Server log lives at `~/.zshrs/zshrs.log` (see [§0x0B](#0x0b-logs)).

---

## [0x04] CODE ACTIONS

Three LSP code actions, all with `kind: "refactor.extract"` so they match `refactor.extract.method` / `refactor.extract.variable` / `refactor.extract.constant` queries via parent-kind matching:

| Action | Selection shape | Edit shape |
|--------|-----------------|------------|
| **Extract to variable** (Cmd-Opt-V) | single-line, full-line OR sub-expression | inserts `local EXTRACTED=<rhs>` above, replaces selection with `$EXTRACTED` |
| **Extract to constant** (Cmd-Opt-C) | single-line, full-line OR sub-expression | inserts `readonly EXTRACTED=<rhs>` above, replaces selection with `$EXTRACTED` |
| **Extract to function** (Cmd-Opt-M) | whole-line OR multi-line | wraps the selection in `extracted_function() { … }` above the block, replaces the original range with a bare call |

Behavior:
- Caret-only invocations snap to the word under the cursor via `snap_to_word_at_cursor` and behave like sub-expression selections.
- Sub-expression selections (mid-line fragments) **only** get Extract Variable / Constant — calling a function for an interpolated value is almost never what you want. Whole-line selections get all three.
- Multi-line selections only get Extract Function; Variable / Constant don't apply to multi-line bodies.
- Function extract preserves relative indentation: strips the block's common leading whitespace, then re-indents one level past the function-decl indent so nested `if` / `for` structure survives.
- Wraps interpolated text in `"…"` when the selection sits inside an open double-quoted or backtick string and isn't already a self-contained expression.

Surfaced under **Alt-Enter** (intentions popup). The IntelliJ Refactor menu (Ctrl-T) routes via `ZshrsRefactoringSupportProvider.kt` so Extract Method / Variable / Constant on the platform's binding all reach the LSP. Cmd-Opt-P (Extract Parameter) has no LSP-side action: zsh functions don't have a parameter list, so Extract Variable into `local NAME=$1` is the canonical workaround.

Pinned by 8 tests under `lsp::tests::code_actions_*`.

---

## [0x05] REFLECTION TOOL WINDOW

*View → Tool Windows → zshrs* (right edge). Tabs populated from `zshrs --dump-reflection`:

| Tab | Source registry | Count |
|-----|-----------------|------:|
| **All** | merged union (last-write-wins on collisions) | 1439 |
| **Builtins** | `ported::builtin::BUILTINS` | 159 |
| **Keywords** | `ported::hashtable::RESWDS` (all reserved words per `man zshmisc`; declaration commands appear in both tabs) | 31 |
| **Options** | `zsh_option_docs::OPTION_DOCS` ∪ `OPTION_ALIASES` (canonical CAPS form per `man zshoptions`) | 756 |
| **Special vars** | `zsh_special_var_docs::SPECIAL_VAR_DOCS` ∪ `SPECIAL_VAR_ALIASES` | 279 |
| **Compsys** | `compsys::COMPSYS_FN_NAMES` (Rust-native `_arguments` / `_files` / `_describe` / per-command completers) | 52 |
| **Extensions** | `ext_builtins::EXT_BUILTIN_NAMES` (91 in-process) ∪ `daemon::builtins::ZSHRS_BUILTIN_NAMES` (23 daemon-backed `z*` builtins) | 114 |

Each tab is a tree with a per-tab search field filtering across name + category.

| Interaction | Effect |
|-------------|--------|
| **Left-click on leaf** | Anchored docs popup. Renders `zshrs --docs NAME` with ANSI colors decoded via IntelliJ's `AnsiEscapeDecoder` + `ConsoleView` — same body as the LSP hover card. |
| **Right-click on leaf** | Context menu: *Show Docs* + *Copy Name* |
| **Toolbar → Refresh** | Re-runs `zshrs --dump-reflection` and reloads every tab |
| **Toolbar → Settings** | Opens *Settings → Tools → zshrs* |

The "Extensions" tab includes daemon-backed `z*` builtins (`zd`, `zcache`, `zls`, `zid`, `zping`, `zlock`, `zpublish`, `znotify`, `zsend`, `zsubscribe`, `zunsubscribe`, `ztag`, `zuntag`, `zsync`, `zjob`, `zask`, `zhistory`, `zsource`, `zcomplete`, `zsuggest`, `zcmd-result`, `zlog`, `zwhere`) — these proxy to `zshrs-daemon` over a Unix socket for cross-shell state.

---

## [0x06] EXTERNAL LIBRARIES

Every zsh plugin that zshrs has sourced appears under **External Libraries** in the Project view — indexable, cmd-clickable, find-usages-able, and renamable across plugin boundaries.

### Data source

The Rust side exposes `zshrs --dump-plugins` (added alongside `--dump-reflection`). It reads the `plugins` table in `~/.zshrs/plugins.db` (or `$ZSHRS_HOME/plugins.db`) — the same SQLite cache that backs sub-millisecond `source` replays in `src/extensions/plugin_cache.rs` — and groups every file path by inferred plugin manager:

| Manager | Path shape | Library name shape |
|---------|-----------|-------------------|
| `zinit` | `…/.zinit/plugins/<user>---<repo>/…` or `…/zinit/plugins/<user>---<repo>/…` | `<user>/<repo>` |
| `oh-my-zsh` | `…/.oh-my-zsh/{plugins,custom/plugins,themes,custom/themes}/<name>/…` | `<name>` (themes get a `.theme` suffix) |
| `prezto` | `…/.zprezto/modules/<name>/…` | `<name>` |
| `antidote` | `…/.cache/antidote/<user>/<repo>/…` or `…/antidote/repos/<user>/<repo>/…` | `<user>/<repo>` |
| `antigen` | `…/.antigen/bundles/<user>/<repo>/…` | `<user>/<repo>` |
| `zplug` | `…/.zplug/repos/<user>/<repo>/…` | `<user>/<repo>` |
| `zsh-more-completions` | `…/zsh-more-completions/…` | `zsh-more-completions` |
| `zpwr` | `…/.zpwr/…` or `…/zpwr/…` | `zpwr` |
| `loose` | anything else | parent-dir basename |

### JSON shape

```json
{
  "schema": 1,
  "plugins": [
    {"manager": "zinit",
     "name": "zsh-users/zsh-autosuggestions",
     "root": "/Users/wizard/.zinit/plugins/zsh-users---zsh-autosuggestions"},
    {"manager": "oh-my-zsh", "name": "git",
     "root": "/Users/wizard/.oh-my-zsh/plugins/git"}
  ]
}
```

### Plugin-side wiring

`com.menketechnologies.zshrs.library.ZshrsLibraryRootProvider` extends `AdditionalLibraryRootsProvider` and returns one `SyntheticLibrary` per plugin entry, with a stable `comparisonId` of `zshrs:<manager>:<name>` so the platform caches library roots across IDE restarts. The Project view tree node label is `<name> (<manager>)`, the location is the absolute root directory, and the icon matches the zshrs file-type icon.

`ZshrsPluginRegistry` (project-scoped service) runs `zshrs --dump-plugins` on `AppExecutorUtil` — never on the indexer thread, which would block the read action. First call returns an empty snapshot and triggers an async fetch; once the fetch lands, `AdditionalLibraryRootsListener.libraryRootsChanged` fires and the platform re-queries the provider.

A `postStartupActivity` (`ZshrsPluginStartupActivity`) kicks off the first fetch at project open so the External Libraries node is populated before the user expands it.

### Refresh

The **Refresh** toolbar button on the **zshrs** tool window (right edge of the IDE) re-runs both `--dump-reflection` (for the tool-window tabs) and `--dump-plugins` (for the External Libraries node). Newly-sourced plugins show up without an IDE restart.

### Empty state

If the user has never run `.zshrc` under zshrs, the `plugins` table is empty and External Libraries shows nothing. The first interactive zshrs session populates the cache; the next IDE Refresh picks it up.

Pinned by `ZshrsPluginDumpParserTest` — 5 tests covering canonical input, missing keys, malformed JSON, and degraded-row dropping.

---

## [0x07] RUN / DEBUG

### Run

| Surface | Behavior |
|---------|----------|
| **Run config** (`ZshrsRunConfigurationType`) | toggles for `-f` / `--no-rcs`, `-x` (xtrace), `-v` (verbose), `--disasm` (fusevm bytecode disassembly), `--dump-ast`; working directory + script args + interpreter args |
| **Context menu** | *Run with zshrs* on any `.zsh` file in the editor or project view; auto-creates a config |
| **Producer** | `ZshrsRunConfigurationProducer` materializes a run config from the active file |
| **Output** | Standard `ConsoleView` — `print` / `echo` / `printf` stream in real time |
| **File → New → Zsh File** | Standard New-File dialog; pick *Script* (shebanged, `set -euo pipefail`, `main` stub), *Function library*, *Rc fragment*, or *Empty*. Same entry surfaces in the Project-view right-click *New* submenu. |

### Debug

DAP-backed, over a loopback TCP socket. Plugin spawns `zshrs --dap 127.0.0.1:<port>`; zshrs connects back.

| Feature | Notes |
|---------|-------|
| Line breakpoints | Gutter toggle / enable / disable; persistent across sessions |
| Continue / Step Over / Step Into / Step Out / Pause / Run to Cursor | Standard XDebugger actions |
| Frames | `file:line` per frame, click to navigate source |
| Variables panel | Scalars, arrays (`@arr`), associative arrays (`%hash`); expandable on click |
| Evaluate dialog | `${var}`, `$(cmd)` command substitutions, `$(( expr ))` arithmetic — resolved against the paused frame |
| Console | Program `print` / `echo` / `printf` streams in real time; DAP `output` events merged with process stdio |

---

## [0x08] DAP PROTOCOL

Plugin side (`com.menketechnologies.zshrs.dap`):

1. `ZshrsDebugRunner.doExecute` opens a `ServerSocket(0)` on `127.0.0.1`, captures the port.
2. Spawns `zshrs --dap 127.0.0.1:<port>` via `KillableColoredProcessHandler` — keeps the process's stdio for Console output, exclusively.
3. Waits up to 10 s for `zshrs` to connect back to the listener.
4. Creates an `XDebugSession` via `XDebuggerManager.startSession` and returns the descriptor via `getMockRunContentDescriptorIfInitialized` reflection — bypasses the platform's split-debugger `Logger.error("[Split debugger] …")` toast that the deprecated `runContentDescriptor` getter fires.
5. `ZshrsDebugProcess.createConsole` builds a `ConsoleView` and `attachToProcess(processHandler)` so program stdout streams in real time.
6. `ZshrsDapClient` reads Content-Length-framed JSON-RPC from the socket — **byte-based, not char-based** — so multi-byte UTF-8 in variable reprs doesn't desync framing.
7. On `stopped` event, `onStopped` synchronously fetches `stackTrace` + `scopes` + `variables`, builds `ZshrsStackFrame` objects with pre-populated children, calls `session.positionReached`. No async expansion on the UI thread — IntelliJ 2026.1's split-debugger drops those.
8. `ZshrsEvaluator` sends `evaluate` requests for the Evaluate dialog.

zshrs side (`src/extensions/dap.rs`):

DAP requests handled: `initialize`, `launch`, `setBreakpoints`, `configurationDone`, `threads`, `stackTrace`, `scopes`, `variables`, `continue`, `next`, `stepIn`, `stepOut`, `pause`, `evaluate`, `disconnect`. Hooks into the shell evaluator at statement boundaries to honour breakpoints + step modes; pulls locals/globals from the current scope chain for the variables view. Same JSON-RPC framing as the LSP server (hand-rolled on `serde_json`, no `dap-types` crate).

---

## [0x09] REFACTOR / RENAME

**Shift-F6** on any of these identifiers renames it across the workspace via `textDocument/rename`:

- Scalar / array / hash variables (`$x`, `@xs`, `%h`) — sigil is part of the extracted identifier, so `$pass` and the `pass` builtin don't collide.
- `local` / `typeset` / `export` / `readonly` declarations.
- Function declarations (`function foo`, `foo()`).
- Aliases (`alias name=…`).

Cross-file rename fires when the symbol crosses document boundaries — the server scans every other open document, finds exact-name matches in its symbol table, and falls back to a textual scan for files that reference the symbol without re-declaring it. Locally-scoped `local` decls and function parameters are file-scoped and never cross files.

Hovering on the key in `$opts[format]` or the selector in `${db[format]}` does NOT show the `format` builtin card — those identifiers are subscripts, not builtin references.

Implementation: plugin handler in `ZshrsRenameHandler.kt`; server-side rename in `src/extensions/lsp.rs::rename`.

---

## [0x0A] CONFIGURATION

*Settings → Tools → zshrs*:

| Section     | Setting                                | Default              | Notes |
|-------------|----------------------------------------|----------------------|-------|
| Interpreter | zshrs executable                       | first `zshrs` on `$PATH` | absolute path or blank |
| LSP         | Enable LSP                             | on                   | master toggle |
| LSP         | Extra LSP args                         | empty                | passed after `--lsp` |
| LSP         | LSP environment                        | empty                | `KEY=VAL` pairs (e.g. `ZSHRS_LOG=debug`) |
| LSP         | Auto-restart LSP on settings change    | on                   | restart picks up new env |
| LSP         | Show builtin hovers                    | on                   | server-provided cards |
| LSP         | Log LSP traffic to file                | off                  | sets `ZSHRS_LSP_LOG=<path>` |
| Editor      | Disable lexer highlighting             | off                  | rely only on LSP semantic tokens |
| Editor      | File extensions                        | `zsh`                | comma-separated; the rc dotfiles always match |
| Run configs | Default new configs to `-f` / `--no-rcs` | off                | skip startup files in new run configs |

Color scheme entries: *Settings → Editor → Color Scheme → zshrs* (**45 sub-categories** grouped under Comments / Strings / Numbers / Keywords / Names / Variables / Operators / Punctuation / Errors).

---

## [0x0B] LOGS

Two append-only logs, both under `~/.zshrs/` (or `$ZSHRS_HOME/` when that env var is set):

| File | Source | Contents |
|------|--------|----------|
| `~/.zshrs/zshrs-plugin.log` | Kotlin (plugin) | LSP command line built, DAP `send` / receive (seq + command + bytes), rename / semantic-token routing, breakpoint handler steps |
| `~/.zshrs/zshrs.log` | Rust (`zshrs --lsp` / `--dap`) | Levelled events (`TRACE` / `DEBUG` / `INFO` / `WARN` / `ERROR`) from both daemons: startup, initialize, every request method (TRACE), didOpen/Change/Close + diagnostics (DEBUG), rename / hover outcomes, DAP launch / breakpoints / step / pause / disconnect, milestone events |

Tail with `tail -f ~/.zshrs/zshrs.log ~/.zshrs/zshrs-plugin.log`.

### Server log level (Rust side)

`$ZSHRS_LOG` accepts the standard `tracing-subscriber` filter syntax — `trace`, `debug`, `info` (default), `warn`, `error`, plus per-module filters like `zsh::lsp=trace,zsh::dap=debug`.

```sh
export ZSHRS_LOG=debug                       # verbose for daily use
export ZSHRS_LOG=trace                       # firehose, every request method logged
export ZSHRS_LOG='zsh::lsp=trace,info'       # LSP at trace, everything else at info
```

Persistent setting (no env var needed each session): set `[log] level = "debug"` in `$ZSHRS_HOME/zshrs.toml`. Env var wins when both are set.

### Redirection

`$ZSHRS_HOME=/abs/path/zshrs-state` moves the entire log + state directory. Per-IDE chatter still goes to `idea.log` via `Logger.getInstance(...)`; the files above are for plugin-and-server events specifically.

---

## [0x0C] BUILDING

```sh
cd editors/intellij
export JAVA_HOME=$(/usr/libexec/java_home -v 17)   # macOS; or set to any JDK 17 install
./gradlew buildPlugin             # → build/distributions/zshrs-intellij-<v>.zip
./gradlew runIde                  # launches a sandbox IDE with the plugin installed
./gradlew verifyPlugin            # plugin verifier against recommended IDE matrix
./gradlew test                    # runs ZshrsLexerTest + ZshrsCommenterTest + ZshrsPluginManifestTest + ZshrsRegistryTest + ZshrsSettingsTest
```

**JDK 17 is required.** Kotlin 2.2.0 (pinned by the IntelliJ Platform Gradle Plugin 2.16) crashes parsing the 3-part version string of newer JDKs (`25.0.2`, `26.0.1`, …) inside `JavaVersion.parse`. Set `JAVA_HOME` to a JDK 17 install before running gradle. The plugin itself targets JVM 17, so any IDE on 2025.2+ runs it.

First build downloads the IntelliJ Platform SDK (~1 GB), takes a few minutes, and is cached under `editors/intellij/.intellijPlatform/` (which is gitignored).

---

## [0x0D] PLUGIN ARCHITECTURE

```
editors/intellij/
├── build.gradle.kts                          # IntelliJ Platform Gradle Plugin 2.16
├── gradle.properties                         # platform version, plugin version, JVM
├── settings.gradle.kts
└── src/main/
    ├── kotlin/com/menketechnologies/zshrs/
    │   ├── ZshrsLanguage.kt                  # Language singleton
    │   ├── ZshrsFileType.kt                  # .zsh + dotfiles → zshrs
    │   ├── ZshrsIcons.kt                     # icon loader
    │   ├── ZshrsColors.kt                    # 45 ZSHRS_* TextAttributesKey constants
    │   ├── ZshrsTokenTypes.kt                # token type enum
    │   ├── ZshrsLexer.kt                     # hand-rolled zsh lexer
    │   ├── ZshrsSyntaxHighlighter.kt         # token → color mapping
    │   ├── ZshrsColorSettingsPage.kt         # IDE color-scheme entries
    │   ├── ZshrsBraceMatcher.kt              # {} () [] [[ ]] (( ))
    │   ├── ZshrsCommenter.kt                 # `#` line comments
    │   ├── ZshrsQuoteHandler.kt              # " ' ` $'…' auto-pair
    │   ├── ZshrsSettings.kt                  # persistent settings
    │   ├── ZshrsSettingsConfigurable.kt
    │   ├── ZshrsDebugLog.kt                  # plugin-side log writer
    │   ├── lsp/
    │   │   ├── ZshrsLspServerSupportProvider.kt
    │   │   └── ZshrsLspServerDescriptor.kt
    │   ├── refactor/
    │   │   ├── ZshrsRefactoringSupportProvider.kt   # Extract Method/Var/Const routing
    │   │   └── ZshrsRenameHandler.kt
    │   ├── navigate/
    │   │   └── ZshrsGotoDeclarationHandler.kt       # Cmd-click + Cmd-B
    │   ├── run/
    │   │   ├── ZshrsRunConfigurationType.kt
    │   │   ├── ZshrsRunConfigurationOptions.kt
    │   │   ├── ZshrsRunConfiguration.kt
    │   │   ├── ZshrsRunConfigurationEditor.kt
    │   │   ├── ZshrsRunConfigurationProducer.kt
    │   │   ├── ZshrsProgramRunner.kt         # Run executor
    │   │   └── ZshrsDebugRunner.kt           # Debug executor (DAP)
    │   ├── dap/
    │   │   ├── ZshrsDapClient.kt             # byte-based DAP protocol client
    │   │   ├── ZshrsDebugProcess.kt          # XDebugProcess
    │   │   ├── ZshrsDebuggerEditorsProvider.kt
    │   │   ├── ZshrsBreakpointType.kt        # xdebugger.breakpointType
    │   │   ├── ZshrsBreakpointHandler.kt
    │   │   ├── ZshrsStackFrame.kt
    │   │   ├── ZshrsSuspendContext.kt
    │   │   ├── ZshrsValue.kt                 # XValue with recursive children
    │   │   └── ZshrsEvaluator.kt             # Evaluate dialog backend
    │   ├── toolwindow/
    │   │   └── ZshrsReflectionToolWindow.kt
    │   ├── library/
    │   │   ├── PluginEntry.kt                # (manager, name, root) record
    │   │   ├── ZshrsPluginRegistry.kt        # project-scoped cache; runs `zshrs --dump-plugins`
    │   │   ├── ZshrsLibraryRootProvider.kt   # AdditionalLibraryRootsProvider → External Libraries
    │   │   └── ZshrsPluginStartupActivity.kt # postStartupActivity: kick off first fetch
    │   └── actions/
    │       └── RunZshrsFileAction.kt
    └── resources/
        ├── META-INF/plugin.xml
        └── icons/zshrs.svg
```

The Rust side lives in:

| Module | Purpose |
|--------|---------|
| `src/extensions/lsp.rs` | LSP server (`zshrs --lsp`) — hover, completion, codeAction, rename, semanticTokens, foldingRange, diagnostics, formatting |
| `src/extensions/dap.rs` | DAP server (`zshrs --dap HOST:PORT`) — breakpoints, stepping, scopes, variables, evaluate |
| `src/extensions/plugin_cache.rs` | `plugins` SQLite table + classification helpers + `dump_plugins_json()` (consumed by the IntelliJ External Libraries view) |
| `src/extensions/zsh_*_docs.rs` | Yodl-derived hover bodies (5 files, ~4400 lines total) — `zsh_builtin_docs.rs`, `zsh_ext_builtin_docs.rs`, `zsh_option_docs.rs`, `zsh_keyword_docs.rs`, `zsh_special_var_docs.rs` |
| `src/extensions/ext_builtins.rs` | `EXT_BUILTIN_NAMES` const (91) — every in-process zshrs-only builtin |
| `daemon/builtins.rs` | `ZSHRS_BUILTIN_NAMES` const (23) — daemon-backed `z*` builtins |
| `src/compsys/mod.rs` | `COMPSYS_FN_NAMES` const (52) — Rust-native completion functions |
| `src/ported/hashtable.rs` | `RESWDS` const — canonical reserved-word table (port of `Src/hashtable.c:1076-1108`) |
| `src/ported/options.rs` | `ZSH_OPTIONS_SET` — canonical setopt registry (197 entries) |
| `src/ported/builtin.rs` | `BUILTINS` — canonical builtin registry (159 entries) |
| `src/ported/lex.rs` + `parse.rs` | Tokenizer + AST used by LSP diagnostics + DAP statement boundaries |

---

## [0x0E] VERSION COMPATIBILITY

Plugin version tracks the zshrs Cargo workspace version. `gradle.properties` controls the supported IDE range via `pluginSinceBuild` / `pluginUntilBuild`. Currently targets the `2025.2` SDK against builds `252..261.*` — every paid JetBrains IDE on **2025.2 +** loads it (RustRover, IDEA Ultimate, GoLand, PyCharm Pro, WebStorm, RubyMine, PhpStorm, CLion, Rider, DataGrip, Aqua). Community editions don't have the LSP API, so the plugin won't load there.

---

## [0x0F] LIMITATIONS

- **No PSI tree** — every symbol-navigation feature (Cmd-click, Cmd-B, Find Usages, rename) routes through the LSP server. Disabling the LSP under Settings disables them all.
- **Debugger v1**: no conditional breakpoints, no hit-count breakpoints, no exception breakpoints, no watch expressions, no Set Value, single-thread only.
- **Lexer is approximate** for arithmetic-expansion `$(( … ))` and deeply-nested expansions. Server-side semantic tokens fill in where the lexer is wrong.
- **`[Split debugger]` toast on Debug start** — the IDE's deprecated `XDebugSession.runContentDescriptor` accessor fires `Logger.error` even when bypassed via reflection if any third-party code touches it during session bring-up. JetBrains' own debug runners suffer the same noise in 2024.3+. Cosmetic only; the debugger works.
- **Extract Parameter (Cmd-Opt-P)** has no LSP-side action. zsh functions don't have a parameter list — the equivalent is `local NAME=$1` inside the body, which Extract Variable already covers.

---

## [0xFF] LICENSE

MIT, same as zshrs.
