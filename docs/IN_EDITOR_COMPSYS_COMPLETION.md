# In-Editor Compsys Completion

**Status**: Design proposal (May 2026)
**Author**: MenkeTechnologies
**Tracks**: zshrs LSP, IntelliJ plugin, future Helix/Neovim adapters

## Problem

Today the zshrs IntelliJ plugin (and any LSP client) completes shell
script tokens through a small, hand-curated path:

- Reserved words (`if`, `for`, `case`) — keyword table
- Builtin names + flags — `BUILTIN_FLAG_DOCS_OVERRIDE` etc.
- Parameter expansions (`${var:h}`, `${(L)var}`) — flag tables
- Glob qualifiers (`*(.r-x)`) — qualifier table
- User-defined functions / aliases / parameters — `scan_symbols`

It does NOT complete:

- **External command names** the way zsh does at the prompt. `g<TAB>`
  in a `.zsh` file gets you nothing; at a zsh prompt it would offer
  `git`, `grep`, `gcc`, `gpg`, every executable in `$path` starting
  with `g`.
- **Subcommand names** for external commands. `git a<TAB>` should
  offer `add`, `am`, `apply`, `archive` — sourced from `_git` (one
  of the most-used compsys functions in existence). Today: nothing.
- **Argument values for known options**. `git add --<TAB>` should
  offer `--all`, `--patch`, `--update`. `kubectl get p<TAB>` should
  query the cluster and offer pod names. `ssh user@<TAB>` should
  read `~/.ssh/known_hosts`. Today: nothing.
- **Dynamic completions** that depend on runtime state — directories
  from `$PATH`, hosts from SSH config, branches from `git`, packages
  from `apt`/`brew`/`pacman`. These are the whole point of compsys.

The hand tables are a stopgap. The user already has the full
compsys ecosystem installed — `_git`, `_kubectl`, `_docker`,
`_systemctl`, `_brew`, plus everything from zsh-completions and
oh-my-zsh and their own plugins. **The LSP should drive that
ecosystem instead of duplicating it.**

## Goal

When the user types in a zsh source file, completions reflect what
zsh's compsys engine would offer at an interactive prompt with the
same line + cursor position.

Concretely:

| User types | Completion offers |
|---|---|
| `g\|` (in a .zsh file) | `git`, `grep`, `gcc`, `gpg`, … (every `$PATH` exe starting with `g`) |
| `git a\|` | `add`, `am`, `apply`, `archive`, … (from `_git`) |
| `git add --\|` | `--all`, `--patch`, `--update`, … (from `_git add`) |
| `kubectl get p\|` | live pod names (from `_kubectl` invoking `kubectl get pods`) |
| `cd ~/Rust\|` | `RustroverProjects/`, `Rusticbuilds/`, … (from `_files` / `_directories`) |
| `ssh user@h\|` | `homelab.local`, `hetzner.example.com`, … (from `_ssh`) |
| `man g\|` | `git`, `gcc`, `grep`, … (from `_man`) |
| `setopt extend\|` | `extended_glob`, `extended_history` (from `_options`) |

These should all "just work" — same matches the user gets at their
real zsh prompt, no plugin-side hand-curation per command.

## Non-Goals

- **Don't replace the existing hand tables.** Builtin docs (with
  full descriptions) stay in the override table — they're better
  than what compsys offers (compsys gives one-line synopses).
  Compsys completion FILLS THE GAP for the 5000+ commands without
  hand tables.
- **Don't run completions for unknown / untrusted commands by
  default.** If the user types `randomcurlcommand <TAB>` and the
  `_randomcurlcommand` completion function isn't in their `fpath`,
  fall back silently. Don't dial random shell commands.
- **Don't compete with shell-interactive completion.** The user
  presses Tab at the prompt for that. This is for editing.
- **Don't reach into other languages.** When the user is editing a
  Python / Rust / TOML file the IDE's native completion runs. This
  is `.zsh` / `.sh` only.
- **Don't slow the editor.** Completion must respect a budget
  (default 200 ms). Slow completions get killed; the user types
  again.

## Why It Works

zshrs is itself a compsys runtime. It ports `_arguments`, `_files`,
`_path_files`, `_describe`, `_wanted`, `_alternative`, `_message`,
`_tags`, `_normal` — the framework that `_git`, `_kubectl`, every
user-installed completion function calls into. The same in-process
machinery that handles a shell-prompt Tab can serve an LSP request.

The only thing missing is the protocol glue: a way for the LSP
client to ask "complete this line at this column" and for the LSP
server to invoke compsys dispatch and return the resulting match
list as `CompletionItem`s.

## Architecture

```
┌─────────────────┐   textDocument/completion   ┌──────────────────┐
│  IntelliJ /     │ ─────────────────────────▶  │  zshrs --lsp     │
│  Helix /        │                             │  (lib.rs)        │
│  Neovim         │ ◀───────────────────────── │                  │
└─────────────────┘   CompletionList            └────────┬─────────┘
                                                         │
                                                         │ in-process call
                                                         ▼
                                                ┌──────────────────┐
                                                │  compsys::       │
                                                │   complete_at(   │
                                                │     line, col)   │
                                                └────────┬─────────┘
                                                         │
                                          ┌──────────────┼──────────────┐
                                          ▼              ▼              ▼
                                  ┌──────────────┐ ┌──────────┐ ┌─────────────┐
                                  │ _path_files  │ │ _git     │ │ _kubectl    │
                                  │ _alternative │ │ (user    │ │ (user       │
                                  │ _arguments   │ │  fpath)  │ │  fpath)     │
                                  └──────────────┘ └──────────┘ └─────────────┘
```

Key properties:

- **In-process.** No subshell spawn. zshrs's LSP server already
  hosts the compsys runtime; calling it costs a function call.
- **fpath-aware.** Reuses the same `fpath` the user's interactive
  shell loaded. New completion functions appear automatically.
- **No protocol change required.** Existing LSP
  `textDocument/completion` already takes line + column. We just
  return more / better items.
- **Cancellable.** LSP clients send a cancellation token on every
  keystroke; long-running completion functions abort cleanly.

### New entry point

```rust
// src/compsys/mod.rs (extend)

pub struct CompsysRequest<'a> {
    /// Whole command line as the user has it typed.
    pub line: &'a str,
    /// 0-based byte column the cursor sits at.
    pub cursor: usize,
    /// Per-call budget. Functions that exceed this are killed.
    pub deadline: Instant,
    /// True when the LSP client wants exec-spawning completions
    /// (`_kubectl get pods` shells out). False = safe mode:
    /// only completions that read `$path`/`$fpath`/static caches.
    pub allow_exec: bool,
}

pub struct CompsysMatch {
    pub completion: String,
    pub description: Option<String>,
    /// Group label from `_tags` / `_describe`. Surfaces as the
    /// IDE's `detail` field (`option`, `value`, `host`, …).
    pub group: Option<String>,
    /// Match start byte in `line` so the LSP can build a
    /// `textEdit` that replaces exactly the typed prefix.
    pub replace_start: usize,
}

pub fn complete_at(req: CompsysRequest) -> Vec<CompsysMatch>;
```

### LSP wiring

In `src/extensions/lsp.rs::handle_completion`:

```rust
fn handle_completion(state: &State, params: &Value) -> Value {
    // 1. EXISTING: try hand-table contexts (BuiltinFlag, ParamFlag,
    //    GlobQualifier, etc.). These win when they match — they
    //    have richer descriptions than compsys offers.
    if let Some(items) = try_hand_table_contexts(state, params) {
        return items;
    }
    // 2. NEW: try compsys dispatch. Covers external commands,
    //    user-defined fpath completion functions, dynamic
    //    completions that depend on runtime state.
    if let Some(items) = try_compsys_completion(state, params) {
        return items;
    }
    // 3. EXISTING fallback: user-defined identifiers from
    //    scan_symbols + plain identifier-prefix matches.
    fallback_identifier_completion(state, params)
}
```

`try_compsys_completion`:

```rust
fn try_compsys_completion(state: &State, params: &Value) -> Option<Value> {
    let uri = params["textDocument"]["uri"].as_str()?;
    let pos = &params["position"];
    let text = state.docs.get(uri)?;
    let line_no = pos["line"].as_u64()? as usize;
    let col = pos["character"].as_u64()? as usize;
    let line_text = text.lines().nth(line_no)?;
    let req = compsys::CompsysRequest {
        line: line_text,
        cursor: col,
        deadline: Instant::now() + Duration::from_millis(state.settings.completion_budget_ms),
        allow_exec: state.settings.allow_exec_completions,
    };
    let matches = compsys::complete_at(req);
    if matches.is_empty() { return None; }
    Some(render_compsys_items(matches))
}
```

## Dispatch Flow

For `git add --p<TAB>`:

1. **Parse**: `compsys::complete_at` re-uses the existing zsh-parser
   to identify words. `words = ["git", "add", "--p"]`, current
   word index = 2, current word = `--p`.

2. **Resolve dispatch function**: walk the `compdef` registry to
   find the function bound to `git`. Standard answer: `_git`. If
   the user's fpath has no `_git`, look for `_default` / `_complete`.

3. **Invoke**: call `_git` with the same context shape it'd see at
   a prompt — `words`, `CURRENT`, `compstate`, the lot. The
   ported compsys runtime in `src/compsys/` already does this for
   `--gen-docs` and the test harness.

4. **Capture matches**: `_git` calls `_arguments` which calls
   `_describe`/`_values`/`compadd`. Intercept the final `compadd`
   calls (they're already going through the ported `compadd`
   builtin in `src/compsys/ported/`); collect the offered
   matches + tags + descriptions.

5. **Marshal to LSP**: each match becomes a `CompletionItem` with:
   - `label`: the match text
   - `detail`: the tag / group / description from `_describe`
   - `kind`: `Variable` for values, `Function` for subcommands,
     `File` for paths
   - `textEdit`: replaces the typed prefix range
   - `sortText`: tag-priority order so `subcommands` show above
     `options`

6. **Return** within the deadline. On timeout: return whatever
   matches collected so far, mark `isIncomplete: true` so the
   client re-requests if the user keeps typing.

## Existing Infrastructure (Re-use)

zshrs already has most of what's needed:

| Have today | Built for |
|---|---|
| `src/compsys/mod.rs` + 50+ ported compsys fns | gen-docs, scriptable completion |
| `_arguments`, `_files`, `_describe` etc. | compsys correctness |
| `compadd` builtin port | accepting matches |
| `fpath` resolution + autoload | full compsys boot |
| The zsh parser (words, redirects, current-word) | shell execution |
| `BUILTIN_FLAG_DOCS_OVERRIDE` table | LSP completion (today) |
| `man zshall` audit test | LSP coverage proof |

We DON'T need to:
- Rewrite `_git` / `_kubectl` — they ship in `$fpath`.
- Build a parser — zshrs has one.
- Define a new protocol — LSP `textDocument/completion` is enough.

We DO need to:
- Add `compsys::complete_at` as a public entry point that returns
  match lists instead of writing to the compsys output stream.
- Add the LSP wiring (`try_compsys_completion`) above.
- Add a budget / cancellation harness so slow completions don't
  block the editor.
- Add an `allow_exec` setting so users can opt in to dynamic
  (subprocess-spawning) completions.

## Performance Budget

| Phase | Budget | What happens |
|---|---|---|
| Parse line into words | < 1 ms | re-use existing parser |
| Resolve dispatch function | < 5 ms | hash-table lookup into compdef registry |
| Invoke static completion (`_options`, `_aliases`) | < 20 ms | pure-Rust over in-process tables |
| Invoke compdef function (`_git add`) | < 100 ms typical, 200 ms hard cap | runs through ported compsys; deadline enforced |
| Invoke `allow_exec` completion (`_kubectl get`) | 200 ms hard cap | spawns subprocess; killable; cached |
| **End-to-end (LSP receive → send)** | **200 ms hard cap, 50 ms typical** | client returns `isIncomplete: true` on timeout |

Caching strategy:
- Per-fpath-fn doc cache: parsed signature for each `_X` cached on first load.
- Per-command result cache: keyed on `(line, cursor, fpath-mtime,
  $PATH-hash)`. Invalidated when any of those change.
- `allow_exec` results cached for 5 s with mtime-based invalidation
  on `~/.kube/config`, `~/.ssh/known_hosts`, etc. — same heuristic
  the shell-history-based interactive completion uses.

## Security / Trust Model

Completion functions can run arbitrary code:

- `_kubectl get pods` runs `kubectl get pods`. Network call. Side
  effects (well, mostly read-only).
- `_npm` runs `npm config get registry`. Reads disk.
- `_curl` reads `~/.curlrc`.
- A malicious `_evil` could run `rm -rf ~/`.

Default position: **opt-in for exec-spawning, on-by-default for
static**. Settings:

```kotlin
// IntelliJ plugin: Settings → Tools → zshrs
allowCompsysCompletion: Boolean = true   // master switch
allowExecCompletions: Boolean = false    // _kubectl, _npm, etc.
trustedFpathDirs: List<String> = []      // ~/.zsh/functions, etc.
```

`allowExecCompletions = false` blocks ANY completion function that
shells out. `allowCompsysCompletion = false` blocks compsys
entirely (just hand-table behavior, today's default).

Trusted-fpath gating: only completion functions found in
`trustedFpathDirs` (or system paths like `/usr/share/zsh/functions/Completion`)
run under `allowExec`. The user's own ad-hoc `_foo` in
`~/scripts/` requires explicit allowlisting before the LSP runs
it under exec.

## Telemetry / Debuggability

The completion timeline is visible in `~/.cache/zshrs/lsp.log`
when `STRYKE_LSP_LOG` is set:

```
[compsys] line=`git add --p` cursor=12 dispatch=_git tags=[options]
[compsys]   _git: 6 matches in 23ms
[compsys]   --patch (option, "Add changes interactively, hunk by hunk")
[compsys]   --prune-empty (option, "…")
[compsys]   ...
[compsys] returned 6 items, isIncomplete=false
```

When a completion times out:

```
[compsys] line=`kubectl get p` cursor=13 dispatch=_kubectl
[compsys]   TIMEOUT after 200ms — process `kubectl get pods -o name` still running, killed
[compsys] returned 0 items, isIncomplete=true
```

## Adoption Plan

**Phase 0 — extract entry point** (~1 week)

Public `compsys::complete_at(req) → matches`. Pure refactor: take
the existing compsys dispatch + `compadd` capture and expose them
as a callable function. No LSP wiring yet.

**Phase 1 — LSP integration, static-only** (~1 week)

Wire `try_compsys_completion` into `handle_completion`. Run with
`allow_exec = false` by default. Verifies the protocol shape end-
to-end before opening exec.

Test fixtures: `git`, `setopt`, `ssh`, `man`, `ls` — all have
upstream compsys functions that work without subprocess spawn.
Audit test (`tests/lsp_compsys_integration.rs`) drives the LSP
with a canned `.zsh` source and asserts each completion appears.

**Phase 2 — exec opt-in** (~3 days)

Settings panel in IntelliJ plugin. `allow_exec` flag plumbed
through. `kubectl`, `npm`, `brew`, `docker` start working when
enabled. Result-cache with mtime invalidation.

**Phase 3 — fpath inspector UI** (~1 week)

IntelliJ tool window listing every compsys function discovered in
fpath, with toggle per function for exec-trust. Right-click →
"Show docs" pops the hand-written description from the function's
top `##` block (LSP doc-hover already does this for `.zsh` files;
re-use the same scanner).

**Phase 4 — Helix / Neovim adapters** (~1 week each)

Both already speak LSP. No code change in zshrs — they get
compsys completion for free once Phase 1 lands. Document the
config-file snippets.

## Risks

| Risk | Mitigation |
|---|---|
| Slow completion functions block editor | Hard 200 ms deadline; client gets `isIncomplete=true`; user can keep typing |
| Malicious / buggy fpath fn runs `rm -rf` | `allow_exec=false` default; trusted-fpath allowlist |
| User's fpath has competing definitions | Standard compsys dispatch already handles this — first match wins, exactly as in interactive shell |
| Completion result differs from prompt completion | Both go through same dispatch + same compadd port. If they diverge, it's a porting bug to fix once, benefits both contexts |
| Editor flicker on long results | LSP `isIncomplete` flag + 5 s exec-result cache absorb keystroke storm |
| Compsys runtime cost on every keystroke | Per-fpath-fn parse cache; per-command result cache. Steady state: hash-lookup + memoized invocation |

## Open Questions

1. **`compstate` mutations.** Compsys functions can set
   `compstate[insert]`, `compstate[list]`, etc. to change the
   *editing* behavior — auto-list, no-insert, menu-completion.
   These don't map cleanly to LSP semantics. Drop them
   (collect matches, ignore compstate side-effects)?
2. **Continuous completion vs Tab-only.** Interactive zsh fires
   completion on Tab. LSP fires on every keystroke (or trigger
   char). Does running `_git` on every key feel right, or do we
   want a "completion trigger" gate (don't run `_git` until the
   user pauses or hits Ctrl-Space)?
3. **Per-document `setopt`.** A `.zsh` file may set its own
   `setopt extended_glob` etc. Should completion honor those, or
   use the LSP-server-wide default?
4. **Completion in `"…"` strings.** `"git $(git $1)"` — should
   the inner `git` get subcommand completion? Per the
   existing string-context gate, today no. With compsys
   integration: probably yes, but it's a UX call.
5. **Multi-line completions.** `git \\\n  add \\\n  --pat<TAB>` —
   the cursor's logical line is the joined three-line `git add
   --pat`. Need a small parser to glue them before dispatch.

## Decision Required

- Is the in-process architecture acceptable, or do you want a
  separate completion daemon (process boundary for security)?
- Phase 0 entry point: ship as `pub fn compsys::complete_at` and
  let the LSP call it directly, OR ship as a JSON-RPC method
  (`zshrs/completeShellLine`) so non-LSP clients can also drive
  it?
- Default for `allow_exec`: off (most cautious) vs on (best UX
  out of the box for users who've already accepted their fpath)?

Recommended defaults: in-process, `pub fn` entry, `allow_exec = on`
gated by trusted-fpath allowlist. Matches what the user's
interactive shell already does — same trust boundary they accepted
when they installed those completion functions in the first place.
