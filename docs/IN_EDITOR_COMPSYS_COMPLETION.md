# In-Editor Compsys Completion

**Status**: Implemented — the LSP serves real compsys matches
(`git ch` → `checkout`, `cherry-pick`; `git checkout ` → branch names)
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

## How it works today

`textDocument/completion` answers from the hand tables first, then
appends whatever compsys proposes for the same line + cursor
(`lsp.rs`, `try_compsys_completion`). The compsys half is the real
engine — the same `docomplete` → `_main_complete` → `_git` →
`_arguments` path a Tab press takes — reached through
`compsys::in_editor::complete_at`.

### The shell thread

One dedicated thread owns the shell (`in_editor::shell_thread`). Two
reasons it cannot be the LSP's own thread:

- `exec::dispatch_function_call` resolves the VM through
  `fusevm_bridge::try_with_executor` / `SESSION_EXECUTOR`, both
  thread-local. Without an executor installed on the calling thread,
  every shell-defined completer silently returns nothing.
- The ported compsys runtime keeps non-reentrant process globals.

Requests are handed over a bounded channel; replies come back on a
per-request channel, so a client that gives up on its deadline cannot
block the thread.

### Bootstrap: rkyv shard, no SQLite

Thread startup builds a `ShellExecutor` (option table, params, env
import) and pours in the daemon's canonical rkyv shard via
`canonical_apply::apply_all` — `~/.zshrs/images/*-recorder.rkyv`,
mmap'd zero-copy. That shard is where the completion state lives:

| Shard field | Effect |
|---|---|
| `compdef` | `_comps[git]=_git` — which function completes which command |
| `fpath` | where completer bodies autoload from |
| `autoload_functions` | `PM_UNDEFINED` stubs in `shfunctab` |
| `zstyle` | the user's completion styles |
| `aliases`, `params`, `bindkeys` | the rest of the recorded environment |

**Prerequisite**: the shard is written by `zshrs record` (recorder →
daemon → `images/`). With no shard, `apply_all` returns 0, the thread
still serves, and matches are limited to what the ported Rust
completers produce with no user state — no `_comps` map means no
`git` dispatch. `in_editor::shard_rows()` reports what was applied.

### Capturing the matches

`COMPADD_CAPTURE_BUFFER` shadows `compadd`: while it is `Some`, the
proposed matches are recorded instead of entering ZLE state. The hook
sits in `bin_compadd` AFTER the flag loop (`complete.rs`, just before
`addmatches`), so it reads the port's own parse — bundled flags
(`-2V-default-`), `-o order`'s argument, `-a` array mode, `-k` keys
mode, `-d` display array, the `-` / `--` terminators. An earlier
version re-parsed the argv itself and mistook `-o nosort`'s argument
for a match.

Query forms are NOT shadowed: `-O name` / `-A name` / `-D name` store
or narrow a parameter and add nothing, so they run for real. `_git`
depends on it — it measures its longest command with `compadd -O
allmatching -a allcmds` and pads every description to that width.

### Per-request state the editor has to fake

- **`compfunc = _main_complete`** — `makecomplist` reads it to choose
  the compsys path; interactively `completecall` plants it from the
  `zle -C` widget, which the editor path skips.
- **The editor line buffer** — `docomplete` re-derives the completion
  buffer from `zle_main::ZLELINE` (char indices), so that is the one
  to write; setting only `compcore::ZLELINE` was overwritten and the
  engine ran against an empty line.
- **Fresh-Tab reset** — `menucmp` / `minfo.cur` / `lastambig` /
  `validlist` / `hasoldlist` are cleared before each dispatch.
  Otherwise `before_complete` reads the previous dispatch's menu state
  and short-circuits into "advance the menu", returning no matches.
- **`COLUMNS=80`, `LINES=24`** — zsh's own no-tty fallback. Completers
  do arithmetic on them: `_git` pads descriptions with
  `${(r.COLUMNS-4.)…}`, which at `COLUMNS=0` clipped every description
  to four characters, and at 200 was slow enough to look like a hang.

### Exec policy

`allow_exec` is on: an editor completion should match the prompt, and
the prompt's `git checkout <tab>` runs `git for-each-ref`. The
deadline is the safety net — `_call_program` spawns the helper with
stdout piped, drains it on a reader thread, and kills it when the
budget expires (`_call_program::run_with_deadline`). With
`allow_exec = false` no subprocess is spawned at all and callers fall
back to their static specs.

Helpers never inherit the editor's fds. `run_lsp` dups the JSON-RPC
endpoints to private descriptors and points 0/1 at `/dev/null`
(`lsp::claim_protocol_fds`): a helper that reads stdin would eat the
editor's requests — the server then sees EOF and exits mid-session,
which is exactly what happened before the fix — and one that writes
stdout would corrupt the frame stream.

### Budget and late results

2 s for the first dispatch (cold `_git` is a 424 KB autoload), 350 ms
after that. An overrun is not lost work: the dispatch finishes in the
background, lands in `in_editor`'s one-entry result cache (3 s TTL),
and the client's next request for the same line is served from memory.
Measured on this tree with a real `_git`: 0.79 s cold, 0.0-0.3 s warm.

### What compsys returns vs what the popup shows

Matches are captured before `addmatches`, so they are NOT filtered by
zsh's matcher specs — the client gets the superset and filters with
`filterText`. That is the right shape for an editor (client-side fuzzy
matching), and it means the list can be wider than the prompt's.
Duplicates are merged in the LSP layer: `compdescribe`'s two-phase add
proposes each word twice, once with a description and once without.

## Backslash continuations are one command

A shell command written across continuation lines is ONE command, and
`text.lines().nth(n)` hands back a fragment that has no command word in
it. Every question the server asks about "what command is this an
argument of" therefore got the wrong answer on the shape completers are
actually written in:

```zsh
_arguments -s \
  '(-v --verbose)'{-v,--verbose}'[be loud]' \
  '*:file:_files'
```

`logical_line_at` joins the chain the way zsh's lexer does — the
backslash and the newline both vanish, the next line's leading
whitespace stays — and maps the cursor into the joined line. It is used
for the compsys dispatch (which otherwise received `'*:file:_files'` as
a whole command line), for the spec context, for hover's spec
exception, and for `lsp_completion_context`, so `print \` + newline +
`  -<tab>` still knows it is completing a flag of `print`.

Item bodies keep using the PHYSICAL line and column: the word being
typed is on the cursor's own line, and that is what an edit range has to
address. A trailing backslash inside a comment is not a continuation
(the comment already ends at the newline), and `\\` is a literal
backslash rather than a continuation.

## Writing specs, not just using them

The other half of an editor's job here is helping the author of a
completer, not only the caller of one. A spec is a quoted string, so the
generic "no completion inside strings" rule suppressed everything —
`'*:file:_fi<tab>'` offered nothing where `_files` is the obvious answer.

`arg_spec_part_at` splits the spec the cursor is in
(`_arguments` / `_values` / `_regex_arguments` / `_alternative`) on its
top-level colons, ignoring the ones inside `[description]`, `(value
list)` and `{eval}` bodies:

| Cursor is in | Offered |
|---|---|
| `-o[desc]` / `*` / `1` — the spec head | nothing (those are the completed command's own option names) |
| `:message:` | nothing (prose shown by `_message`) |
| `:action` first word | the inline action forms — `(list)`, `((val\:desc))`, `->state`, `{eval}`, message-only — then every completer this shell knows, `_files` / `_directories` / `_normal` first |
| inside the action's own `(…)` / `{…}`, or on an argument of it (`_files -g …`) | nothing |

Hover follows the same rule: `_files` named in an action gets its
`man zshcompsys` card, where the generic string gate used to suppress it.
That card also stopped calling compsys functions "zsh builtin" — the doc
table is one flat map over every `man zsh*` item, so `_arguments` sat in
it beside `print`, and the label was simply wrong on the card an author
sees while writing a completer.

The completer list comes from the ported table plus every `_`-prefixed
function in `shfunctab`, which on a machine with a completion corpus
installed is the corpus itself — the stubs the canonical rkyv shard
registered.

## Autoload chunk cache

The dominant cost of a cold in-editor completion is parsing the completer,
not running it: `_git` is 424 KB of shell. The loader
(`vm_helper::run_autoload_definition`) therefore caches the compiled
DEFINITION PROGRAM — the chunk for `name() { <file body> }` as
`autoload_register_source` builds it — in `~/.zshrs/autoloads.rkyv`, and
on the next process runs that chunk instead of lexing, parsing and
compiling the file again.

Correctness comes from what is cached and what is stamped:

- The cached chunk is the same one the non-cached path compiles and runs,
  so the installed function is identical by construction — including
  `$LINENO`'s base and `funcsourcetrace`, which is where a
  wrong-shaped chunk shows up first. Verified against `zsh -f`:
  `lineno=2`, `trace=<file>:0` for a body whose first statement is on
  line 2, cold and cached alike (`tests/autoload_chunk_cache.rs`).
- Each entry carries the definition file's mtime AND byte length. The
  binary mtime alone (the pre-existing check) only catches a zshrs
  rebuild; editing `~/.zsh/functions/_foo` leaves it untouched, and the
  length catches a same-second edit on a coarse-mtime filesystem.
- Caching is declined when the compiled program is not a function of the
  file's bytes alone: `ksh_autoload` style (the file runs at top level
  instead of being wrapped, so a runtime option changes the program) and
  `autoload` without `-U` (the body is parsed WITH alias expansion, so
  the chunk depends on the alias table).

Shard format v2. v1 stored the bare file body compiled as a top-level
script, which nothing ever read — the only caller of
`autoload_cache::try_load` was `dbview` — and which would have installed
a different program than the loader does. The version bump discards those
entries, and the two speculative pre-warm paths that produced them (one
fed from the SQLite mirror, one an opt-in loop inside the compinit
background scan) are gone: the write-through cache fills exactly the
functions a session actually calls, with chunks that are correct by
construction.

## Remaining work

- **Quoted strings spanning a raw newline.** `logical_line_at` joins
  backslash continuations, not a single-quoted word that simply runs
  across lines (`'…<newline>…'`). Rare in specs, wrong when it happens.
- **The spec head.** `'-<cursor>[desc]:…'` offers nothing because the
  option names belong to the command the completer is FOR. A completer
  file names that command in its `#compdef` header, so the server could
  dispatch compsys for it and offer its real options — the same trick
  the completion engine already performs, pointed at the file being
  edited.
- **One-entry result cache.** A dispatch that overran its budget is
  servable to the immediately-following request only. Keying by
  (command, word) would survive a keystroke storm.
- **Matcher-spec filtering.** Matches are captured before `addmatches`,
  so the client gets a superset and filters it. Deliberate (client-side
  fuzzy matching is better in an editor), but it does mean the list can
  differ from what the prompt would show.

## Adoption Plan

Phases 0-2 are done (see "How it works today"): the entry point ships
as `pub fn compsys::in_editor::complete_at`, the LSP drives it from
`textDocument/completion`, and exec is on with a deadline rather than
an opt-in toggle. `tests/lsp_compsys_editor.rs` is the hermetic
regression test — it writes a synthetic recorder shard into a temp
`$ZSHRS_HOME` pointing at a fixture completer, so it never reads the
developer's `~/.zshrs` and passes on a machine with no shard.

Still open:

**fpath inspector UI** — IntelliJ tool window listing every compsys
function discovered in fpath, with per-function exec trust. Right-click
→ "Show docs" pops the function's top `##` block (the LSP doc-hover
scanner already extracts it).

**Helix / Neovim adapters** — both already speak LSP and get compsys
completion with no zshrs change. Needs the config snippets documented.

## Risks

| Risk | Mitigation |
|---|---|
| Slow completion functions block editor | 350 ms steady deadline (2 s first dispatch); client gets `isIncomplete=true`; the overrunning dispatch lands in the late-result cache so the retry is instant |
| Malicious / buggy fpath fn runs `rm -rf` | Same trust boundary as the user's own prompt — these are the completers already installed in their fpath. Helpers are killed at the deadline and get `/dev/null` for stdin/stdout |
| User's fpath has competing definitions | Standard compsys dispatch already handles this — first match wins, exactly as in interactive shell |
| Completion result differs from prompt completion | Both go through same dispatch + same compadd port. If they diverge, it's a porting bug to fix once, benefits both contexts |
| Editor flicker on long results | LSP `isIncomplete` flag + 3 s late-result cache absorb the keystroke storm |
| Compsys runtime cost on every keystroke | Per-fpath-fn parse cache; per-command result cache. Steady state: hash-lookup + memoized invocation |

## Open Questions

Settled:

1. **`compstate` mutations** — collected matches only; the editing-side
   side effects (`compstate[insert]`, `[list]`, menu behaviour) are
   dropped, and the interactive continuation state they leave behind is
   reset before every dispatch.
2. **`allow_exec` default** — on, bounded by the request deadline.
3. **Entry point** — `pub fn`, called in-process by the LSP. No
   separate daemon, no JSON-RPC method for other clients yet.

Still open:

1. **Continuous completion vs Tab-only.** Interactive zsh fires on Tab;
   LSP fires per keystroke. Running `_git` on every key is affordable
   warm (60 ms) but not free — a trigger gate (pause / Ctrl-Space)
   may still be worth it.
2. **Per-document `setopt`.** A `.zsh` file may `setopt extended_glob`
   for itself. Completion currently uses the recorded environment, not
   the document's own options.
3. **Completion in `"…"` strings.** `"git $(git $1)"` — the inner `git`
   is gated off today by the string-context check.
4. **Multi-line continuations.** The cursor's logical line spans the
   backslash continuations; they are not glued before dispatch.

## Decisions taken

- **In-process**, on a dedicated shell thread — not a separate
  completion daemon. The thread-local executor requirement makes the
  thread boundary mandatory; the process boundary bought nothing the
  deadline + `/dev/null` fds do not already cover.
- **`pub fn` entry point** (`compsys::in_editor::complete_at`), called
  directly by the LSP. A JSON-RPC method for non-LSP clients can wrap
  it later without changing the engine.
- **`allow_exec = on`**, bounded by the request deadline: same trust
  boundary the user accepted when they installed those completers, and
  the only way `git checkout <tab>` can offer branch names.
