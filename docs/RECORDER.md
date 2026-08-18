# RECORDER.md — Plugin-Framework-Agnostic State-Modification Recorder (PFA-SMR)

**Status:** design  
**Owner:** MenkeTechnologies  
**Layer:** runtime AOP intercept + daemon catalog + query builtin  
**Codename:** the *recorder*  
**Critical non-functional constraint:** **zero recorder code in the default `zshrs` binary, period.** Not "zero overhead at runtime" — *zero bytes*. The recorder is a Cargo feature flag (`recorder`), off by default. The default `zshrs` binary is compiled without `--features recorder`, so every recorder aspect, every dispatcher swap, every IPC type, and every query-side code path is `#[cfg(feature = "recorder")]`-gated and deleted by the compiler before `objcopy` runs. A separate `zshrs-recorder` binary is built with the feature on. `objdump -d target/release/zshrs | grep -c recorder` must return `0`. See "Zero-overhead default binary" below.

## What this document is

A design for the **Plugin-Framework-Agnostic State-Modification
Recorder** (PFA-SMR, or simply "the recorder"): a one-shot indexing
pass over a user's shell init that captures every state modification
— alias, function definition, export, fpath append, hash -d, zstyle,
bindkey, compdef, zmodload, setopt, trap, sched — with the **exact
source file and line number** where each modification happened, plus
the function call-stack at the time. Records flow to `zshrs-daemon`,
which stores them in SQLite + a rkyv shard for read-only mmap'd
lookup from any future shell.

The result: `zwhere alias gst` answers
`~/.zpwr/env/.shell_aliases_functions.sh:1742 (zpwrLoadAliases ← _zpwr_init)`
instantly, regardless of how `gst` was loaded — zinit, oh-my-zsh,
antigen, raw `source`, or inline in `.zshrc`.

### The granularity gap: per-plugin diff vs per-definition record

`zinit @zinit-report` is a **before/after state diff at the plugin
boundary**. zinit takes a snapshot of the shell state before sourcing
a plugin, sources the plugin (allowing it to do whatever it wants
internally), takes another snapshot after, and reports the
delta — function names that exist that didn't before, variables that
changed value. Three structural properties of this approach:

1. **Granularity is the plugin, not the definition.** A plugin
   that defines 12 aliases, 8 functions, 3 bindkeys, 4 zstyles, and
   2 fpath edits produces ONE report entry. The 29 individual state
   mutations are collapsed into a single "here's what this plugin
   added overall" summary. The recorder produces 29 records, each
   with its own file:line, fn_chain, and timestamp.
2. **Temporal order WITHIN a plugin is lost.** Even if a plugin
   defines `alias x=A` then conditionally redefines it as `alias x=B`
   based on an internal check, the diff shows only the final value.
   The recorder shows both definitions in order with the override
   chain (`prev_def_id`).
3. **The plugin boundary itself is the ATTRIBUTION unit.** zinit
   can say "MenkeTechnologies/zsh-z added function `zshz`." It
   cannot say which line of which file inside that plugin defined
   `zshz`. To zinit, "the plugin" is opaque; only the name + version
   + load options are tracked. The recorder's attribution unit is
   the EXACT call site of the dispatcher, file:line:fn_chain, with
   plugin-name-as-context (recoverable from `parent_paths_json` in
   the daemon's source resolver).

**The recorder's per-definition granularity is the novel claim.**
zinit-report's per-plugin diff is well-known prior art going back to
~2017; nobody has shipped per-definition file:line attribution
across every state mutation in any shell ecosystem. Even WITHIN
zinit's own scope (plugins zinit knows about), zinit can't deliver
file:line because it doesn't intercept dispatchers — it diffs
namespaces. Diffing namespaces fundamentally cannot recover the
provenance of individual mutations; only intercepting at definition
time can.

The recorder is therefore strictly more granular than zinit-report
even on zinit's home turf, and it covers ground zinit can't reach
(non-zinit-loaded code, deferred mutations from other plugins,
`eval`-generated definitions, mode-conditional branches).

### Concrete contrast: what zinit's report *is* vs what the recorder *delivers*

`@zinit-report` for a single plugin (verbatim):

```
Report for MenkeTechnologies/zsh-z plugin
-----------------------------------------
Source zsh-z.plugin.zsh (reporting enabled)
Autoload is-at-least with options -U
Autoload add-zsh-hook with options -U
Autoload _zshz_precmd
Autoload _zshz_chpwd

Functions created:
_zshz_add_path         _zshz_chpwd
_zshz_find_common_root _zshz_find_matches
_zshz_legacy_complete  _zshz_output
_zshz_precmd           _zshz_remove_path
_zshz_update_datafile  _zshz_usage
zsh-z_plugin_unload    zshz

Variables added or redefined:
ZSHZ  [ "" -> association ]

Completions:
_zshz [enabled]
```

What that report tells you and what it does not:

| zinit @zinit-report has | recorder will deliver |
|---|---|
| Names of created functions | Names of created functions |
| Names of redefined variables | Names of redefined variables, with diff |
| Plugin-source path (one level) | Plugin source AND every transitive `source` it triggered |
| **Per-plugin granularity (one entry per plugin)** | **Per-definition granularity (one record per `alias` / `function` / `bindkey` / etc. call)** |
| **Temporal order within a plugin: lost** | **Temporal order: `order_idx` per mutation in load sequence** |
| **Intra-plugin override chain: lost** | **Intra-plugin override chain: `prev_def_id` chain reachable** |
| **No file:line for any function** | **`/Users/wizard/.zinit/plugins/MenkeTechnologies---zsh-z/zsh-z.plugin.zsh:412` for `zshz`, `:288` for `_zshz_add_path`, etc.** |
| **No function call-stack at definition time** | `fn_chain: zinit-load → MenkeTechnologies/zsh-z::zsh-z.plugin.zsh` |
| **No timestamp / load-order** | `order_idx` per definition within the run + `ts_ns` |
| **No override / shadow chain** | "this `zshz` definition shadows an earlier one at `OMZL::z.zsh:88`" via `prev_def_id` |
| **Only zinit-loaded plugins** | Every state mutation regardless of loader: zinit, OMZ, antigen, sheldon, manual `source`, raw inline `alias x=y` in .zshrc |
| **Specific to one plugin manager** | Plugin-framework-agnostic by construction — no plugin-manager code is consulted, only the runtime's builtin dispatchers |
| Stops at zinit boundary | Continues across plugin-manager boundaries: also catches `ZSHZ_DATA=...` from plugin code, `compdef _zshz zshz`, `zstyle :completion:*:zshz ...`, `bindkey '^z' zshz-widget`, every PATH/fpath edit, every setopt change |

## Why this exists

### The problem with static analysis

The daemon used to do AST-based static analysis (the `ast_walker` /
`walk` / `plugin_walk` / `zshrc_analysis` pipeline, since removed — see
`daemon/lib.rs`) to catalog the user's shell config. But static
analysis hits a hard ceiling:

- **Plugin frameworks rewrite control flow.** Zinit-turbo defers
  loading via `wait` ICE; oh-my-zsh chains `source` calls through
  `OMZ::lib/*`; antigen wraps everything in functions; sheldon does
  template-based inclusion. Each framework imposes its own
  metaprogramming layer that's hostile to AST walking.
- **Dynamic constructs can't be resolved without execution.**
  `eval $(starship init zsh)`, `source <(kubectl completion zsh)`,
  `[[ $TERM == xterm* ]] && alias x=...`, `for f in $ZDOTDIR/*.zsh; do source $f; done` — none of these yield definite information from a static walker. We hit this in the daemon's former `ast_walker` pipeline (since removed — see `daemon/lib.rs`).
- **Conditional load matters.** `if [[ -d ~/.rbenv ]]; then source ...; fi` produces different state on different machines. Static analysis must assume both branches; runtime knows which actually fired.
- **Plugin-manager coupling is non-monotonic.** Even if we wrote a perfect zinit static analyzer today, it'd break on the next zinit ICE feature, on the next oh-my-zsh version that introduces new sourcing primitives, or on the user's hand-rolled framework. There is no end-state to that work — every framework version is a new static-analysis target.

The static path has been the standard answer (zinit's `@zinit-report`, antibody's `bundle list`, oh-my-zsh's `omz changelog`) and it has the same limitation everywhere: it can only report what it itself loaded, and it can't tell you the file:line where a definition lives.

### The runtime answer

Run the user's actual init under instrumented zshrs. Every time a state-mutating builtin executes, capture (kind, name, value, file, line, fn_chain, timestamp) and forward it to the daemon. The shell already has the lineno and file context — `src/ported/parse.rs` plumbs it through `ZshPipe.lineno` and `crate::ported::lex::toklineno()`. We just need to carry it across the builtin dispatch boundary and emit a record.

This is plugin-framework-**agnostic by construction**: it doesn't matter how the alias got defined — through 5 layers of zinit ice modifiers or in raw .zshrc — the `bin_alias` dispatcher fires once per definition, with the file:line at the moment of execution. The recorder catches all of them uniformly.

## Why this can only exist in zshrs: global AOP as the prerequisite

The recorder is not a free-standing capability that any shell could
add. It is structurally **dependent on global runtime AOP across
every builtin dispatcher** — a property zshrs is the **first Unix
shell ever to have**.

### What "global AOP" means here, and why no other shell has it

Aspect-oriented programming (AOP) in the strict sense — pointcuts,
join points, before/around/after advice, aspect weaving across the
union of dispatcher entry points — is a research-language feature
historically (AspectJ for Java, AspectC, CLOS `:around` methods in
Common Lisp). It has never been implemented in a Unix shell prior
to zshrs. The closest precedents in the shell space are all
strictly weaker:

| Shell | Closest analog to AOP | What it actually is | Why it isn't AOP |
|---|---|---|---|
| bash | `trap '...' DEBUG` / `RETURN` | Per-command before/after hook | Single global hook, no per-builtin granularity, no around-advice, no aspect composition |
| bash | `PROMPT_COMMAND` | Pre-prompt callback | Single hook at one point, not weaving |
| zsh (C) | `preexec` / `precmd` / `chpwd_functions` / `periodic_functions` | User-space hook arrays | Fixed hook points only (4-5 named locations); no aspect at the dispatcher level; no `alias`/`compdef`/`bindkey`-level weaving |
| zsh (C) | `zsh/zselect` add-zsh-hook | Hook registration helper | Sugar over the same fixed hook points |
| ksh | `typeset -f` `.sh.fun_get` discipline | Per-variable get/set discipline | Limited to variable access, not builtin dispatch |
| ksh | `DEBUG` trap | Same as bash | Same limitations |
| fish | `function --on-event ...` | Named-event subscription | Hooks at named events only (fish_prompt, fish_postexec, etc.); no dispatcher-level weaving |
| fish | `function --on-variable ...` | Variable-change watcher | Variable scope only |
| nushell | Plugins via Wasm / external subprocess | Out-of-process extensions | Not in-runtime; cannot intercept dispatcher entry, only consume input/produce output |
| elvish | Module-level overrides | Function-level shadowing | User-space shadowing, not aspect weaving |
| pwsh | `*Variable -Force` overrides + `Trace-Command` | Function overrides + execution tracing | Override is shadow-not-around; trace is unstructured stderr |

None of these can intercept the union of state-mutating dispatchers
(alias / typeset / function-decl / compdef / bindkey / zmodload /
setopt / hash / zstyle / source / trap / sched) with structured
metadata emission, around-advice that delegates to original logic,
and per-aspect feature gating. That capability simply does not exist
in any deployed Unix shell.

### Why C zsh cannot retrofit this

zsh's existing C codebase has the structural blocker the audits
already identified: the parser/lexer/builtin-dispatch layer is
hand-coupled global state with no clean intercept seams.
`bin_alias` in `Src/builtins.c` is called directly from the
executor with no indirection point that could host advice. Adding
AOP would require:

- An indirection layer at every builtin dispatch (currently direct
  function call)
- A per-dispatcher aspect chain data structure
- Compile-time conditional compilation of advice with no runtime
  overhead when disabled (otherwise the cost is borne by every
  shell, not just recording shells)
- Cross-aspect composition rules (what if multiple aspects target
  the same dispatcher? in which order? around vs before vs after?)
- Re-entrancy guards (an aspect must not recursively re-fire when
  the advice body itself calls a builtin)

Each item is a non-trivial refactor of decade-old C code with
zero unit tests. The cumulative cost is multi-year and the
maintenance burden is permanent. zsh's volunteer-time-only
maintenance model can't absorb it. fish's Rust rewrite hasn't
attempted it. nushell isn't designed for it.

zshrs gets it for free because the rewrite was clean-slate. The
dispatch layer is already function-pointer-style indirection
(required for the worker-pool / parallel-execution / fusevm-JIT
architecture); aspects compose via the same mechanism that the
worker pool uses. AOP costs zshrs almost nothing structurally
because the prerequisite indirection was already there for other
reasons.

### Stacked structural moats

This makes the patent and competitive landscape a TWO-layer moat,
not one:

```
                Layer 1 (foundational, world-first):
                ┌────────────────────────────────────┐
                │ Global AOP across builtin          │
                │ dispatchers in a Unix shell        │
                │   (zshrs, ~2025)                   │
                └─────────────────┬──────────────────┘
                                  │ enables
                Layer 2 (derivative, world-first):
                ┌─────────────────▼──────────────────┐
                │ Plugin-Framework-Agnostic          │
                │ State-Modification Recorder        │
                │   (zshrs, ~2025)                   │
                └────────────────────────────────────┘
```

**A competitor wanting to ship the recorder must first ship global
AOP in their shell.** That's a multi-year prerequisite project
before they can begin the recorder. The combination is therefore
defended structurally, not just legally.

### Patent-strategy implication

Aligning with the existing `aot_patent_strategy.md` memory: the
"AOP intercepts" line item under claim A or C should be elevated
to its own dependent claim explicitly:

- **Claim X.Y (new dependent claim under whichever omnibus covers
  the runtime):** Method for global aspect-oriented programming in a
  Unix shell, comprising indirection-pointer dispatch at the union of
  state-mutating builtin entry points (alias, typeset, function
  declaration, compdef, bindkey, zmodload, setopt, hash, zstyle,
  source, trap, sched), enabling before/around/after advice
  composition with re-entrancy guards and feature-gated removal in
  default builds.
- **Claim X.Z (dependent on X.Y):** The Plugin-Framework-Agnostic
  State-Modification Recorder of (this design doc), made possible by
  X.Y, providing structurally universal shell-state attribution
  forward-compatible with arbitrary plugin frameworks and shell-mode
  configurations.

Two stacked dependent claims, each with its own demonstrably-novel
substrate. The recorder claim depends on the AOP claim;
both are world-firsts in the Unix-shell category.

### Why the AOP-prerequisite makes this argument bulletproof

Anyone disputing the recorder's novelty has to dispute on one of
two grounds:

1. "The recorder isn't novel; X already does this." Then X must have
   global AOP across builtin dispatchers, OR achieve the recorder's
   functionality without it. The first is empirically false (no
   existing shell has it); the second is structurally impossible
   (there is no other entry point through which all state mutations
   pass).
2. "Global AOP isn't novel for a Unix shell; Y already had it."
   Then Y must have implemented before/around/after advice
   composition at the dispatcher level. No deployed Unix shell has;
   the comparison table above is exhaustive of the current shell
   landscape.

Both grounds collapse on inspection. The combination of
(global-AOP × universal-dispatcher-coverage × cross-shell-shared-via-daemon × file:line-precise-attribution × forward-compat-by-architecture × shell-mode-quirk-elimination × zero-overhead-default-binary)
is unique to zshrs. The recorder is the demonstration, but the
structural moat is the AOP layer beneath it.

## Universal coverage — every zsh plugin framework, past and future

The "plugin-framework-agnostic" property in the name is not marketing
hedging; it's a structural guarantee derived from where the recorder
intercepts.

### The mathematical bottleneck

zsh has a finite, stable set of state-mutating primitives. Aliases
flow through the alias-builtin dispatcher. Functions are created by
either function-definition syntax (which compiles to `WC_FUNCDEF`) or
by `autoload`. Variables flow through `typeset`/`declare`/assignment-
syntax (which compiles to `WC_ASSIGN`). Completions through `compdef`.
Key bindings through `bindkey`. Module loads through `zmodload`.
Options through `setopt`/`unsetopt`. Hash dirs through `hash`.
Heredoc-based environments through expansion sites that run those
same dispatchers.

Every zsh plugin manager — past, present, and future — eventually
calls these dispatchers, because there is no other way to install
state into a zsh session. A plugin framework can:

- generate the calls inline (raw `source`)
- defer them with hooks (zinit `wait` ICE, oh-my-zsh `lazy`-load)
- rewrite them through wrappers (antigen's bundle abstraction)
- queue them in arrays (zgen's compile-once cache)
- code-generate them at install time (sheldon's templating)
- fork them across subshells (zplug's parallel mode)
- invoke them via `eval $(generated_code)` (starship init, kubectl-completion)
- emit them as a single concatenated `.zwc` blob (zinit-turbo's
  compile mode)

…but in **all** cases the actual state mutation happens at the
dispatcher level. Whatever happens upstream — ICE modifiers, deferred
loaders, hook chains, compile-time rewrites, conditional sourcing —
is irrelevant to the recorder. It sees the dispatcher fire, with the
file:line of the call site at the moment it fires, and records it.

### What the recorder catches across plugin-framework history

The pre-existing plugin-manager landscape and what the recorder
captures from each, all from the SAME 15 aspects on dispatchers:

| Era | Framework | Mechanism the recorder is blind to | What the recorder still catches |
|---|---|---|---|
| 1990s-2000s | raw `source` from `.zshrc` / `.zshenv` | (no abstraction) | Every alias/function/export/setopt/etc. with file:line |
| ~2010 | antigen | `bundle` wraps `source` in helper functions | Every alias/function/etc. — recorded under the helper's source line in fn_chain |
| ~2014 | zgen | Concatenates plugin sources into a single cache file | Every definition — file:line points into the concatenated cache, fn_chain shows the zgen helper that loaded it |
| ~2014 | zplug | Parallel sourcing via background subshells | Each subshell runs its own dispatcher; recorder receives events in load order |
| ~2015 | prezto | Module-init pattern with rich dependencies | Every definition — fn_chain shows the prezto module's init function |
| ~2015 | oh-my-zsh | Theme + plugin sourcing chain through `OMZ::lib/*` | Every definition with full fn_chain through OMZ helpers |
| ~2016 | antibody | Static bundle list resolved into `source` calls | Same as raw `source` — file:line of each plugin file |
| ~2017 | zinit | `wait` ICE, `lucid`, `atload`, `as`, `from`, `pick`, dozens of modifiers + turbo mode | Recorder doesn't know zinit exists — captures every `alias` / `compdef` / `bindkey` after zinit-turbo's deferred subshell flushes them through dispatchers |
| ~2018 | znap | Zsh-snap; in-memory cache, lazy load | Every definition with file:line + fn_chain |
| ~2020 | zsh-defer / fast-syntax-highlighting's loader | Deferred-execution loaders | Recorder sees mutations at flush time |
| ~2021 | sheldon (Rust) | Cargo-style template generation, compiles to a single zsh init script | Every alias/etc. recorded with file:line in the generated init script |
| ~2022 | zsh4humans | Heavy hook + turbo abstraction | Same — recorder sees every dispatcher fire |
| ~2024+ | future framework X | Whatever metaprogramming layer X invents | Will hit a dispatcher to install state. Caught. |

For frameworks that pre-load via concatenation/compilation (zgen,
zinit-turbo, sheldon), the recorded `file` is the concatenated/
compiled artifact rather than the user's per-plugin source. That's
correct — the actual state mutation IS in the artifact at runtime.
The daemon's source-resolver layer can map back to the original
plugin source via `parent_paths_json` (already wired in
`source_resolver.rs:135`), so the lineage from artifact → original
plugin file → user's plugin manifest is queryable too.

### What about non-zsh metaprogramming?

Plugin managers written in Rust (`sheldon`), Python (`zinit-zsh-py`
prototypes), Go, etc. ultimately produce zsh code that runs in a zsh
session. The recorder catches the resulting zsh executions. If a
hypothetical framework instead manipulated zsh internals via FFI / a
custom builtin loaded via `zmodload`, the recorder would still catch
the `zmodload` call itself (which is in the aspect set), and
post-load any state changes the module makes via the public builtin
APIs would also flow through the dispatchers. The only way to evade
the recorder is to write a zsh module that mutates internal zsh
state directly without going through the public dispatcher API —
which is (a) something no real plugin manager does, (b) requires C
code shipped with the user's shell, and (c) would break across zsh
upgrades. Not a practical evasion path.

## Shell-mode-quirk elimination

The same structural property that makes the recorder
plugin-framework-agnostic also makes it shell-mode-quirk-free.
Static analysis tools have to model — or fail to model —
zsh's mode matrix:

| Axis | Possible modes | Effect on what gets sourced / what runs |
|---|---|---|
| TTY | interactive (`-i`) vs non-interactive | `.zshrc` only sourced when interactive; ZLE only active interactively; many plugins gate on `[[ -o interactive ]]` |
| Login | login (`-l`) vs non-login | `.zprofile` / `.zlogin` only on login; some plugin managers initialize differently |
| Sandbox / restricted | `zsh -r` (rzsh), `--no-rcs`, `--no-globalrcs`, custom seccomp profiles | Some builtins disabled; some `source` calls fail silently; conditional plugin loads short-circuit |
| Command mode | `zsh -c 'code'` vs full shell | `.zshrc` skipped unless `--rcs` explicitly enabled |
| Subshell | parent vs forked subshell vs `coproc` | Some env state reset; some plugin frameworks reload in subshells, others don't |
| `$TERM` | xterm/screen/dumb/Apple_Terminal/etc. | Conditional `case $TERM` branches gate prompt features, key bindings, color enables |
| `$ZSH_NAME` / `$0` | how the binary was invoked | Some configs (`~/.zshrc.local`, framework switch logic) gate on this |
| Options at startup | `setopt`/`unsetopt` from defaults / global zshrc | Affects parser behavior (KSHARRAYS, SHGLOB), expansion (RC_QUOTES, BARE_GLOB_QUAL), command-not-found behavior, etc. |
| `-o emulate` mode | zsh / sh / ksh / csh emulation | Different reserved-word handling, different default options, different parameter expansion semantics |

Static analysis has to predict the outcome of `if [[ -o interactive ]]
&& [[ -t 0 ]]`, `case $TERM in xterm*)...`, `[[ -n $SSH_CONNECTION ]]`,
etc. Any prediction is wrong on the configurations where the prediction
disagrees with reality. The fundamental problem: **the configuration
that determines what gets loaded is itself part of the input being
loaded**, so you cannot resolve it without running it.

The recorder runs it. There is no prediction; there is only
observation.

### How mode-quirks dissolve under runtime AOP

The runtime evaluates every conditional in its actual environment.
The dispatchers fire only on the branches that actually execute. The
recorder records exactly what was installed and nothing it predicted
might be.

Concretely:

```zsh
# Some plugin file, hostile to static analysis:
if [[ -o interactive ]] && [[ "$TERM" == xterm* ]] && (( $+commands[fzf] )); then
    bindkey '^R' fzf-history-widget
    alias fzf-friendly-ls='ls --color | fzf'
    fpath+=(/usr/share/fzf/completions)
elif [[ -n "$SSH_CONNECTION" ]]; then
    alias remote-prompt-tweak='print "ssh!"'
fi
```

Static analyzer must guess. The recorder records:

- Run the recorder in your interactive xterm with fzf installed → 3
  records (bindkey, alias, fpath_mod) at that file:line, fn_chain
  from your config.
- Run the recorder over SSH → 1 record (the `remote-prompt-tweak`
  alias) at the elif branch's file:line.
- Run the recorder under `zsh -c 'source ~/.zshrc'` (non-interactive)
  → 0 records from this snippet (interactive guard fails); maybe
  records elsewhere if the file is sourced at all.

Each recorder run accurately reflects its actual environment. No
modeling needed.

### Per-mode tagging

The recorder captures the mode it ran in as part of each `runs` row:

```sql
ALTER TABLE runs ADD COLUMN modes TEXT;
-- e.g. "interactive,login,xterm,fzf-installed"
-- or "non-interactive,command-mode"
-- or "interactive,login,sandbox=rzsh"
```

`zwhere` can filter:

```
zwhere alias gst                           # current run's records
zwhere -m interactive alias gst            # only from interactive runs
zwhere -m sandbox=rzsh alias gst           # only from rzsh sandbox runs
zwhere -m diff interactive non-interactive # what's in interactive that's not in non-interactive
```

The diff form is itself a debugging superpower: "show me what my
shell config does in interactive mode that it doesn't do in
non-interactive mode" answers questions zsh users could not
previously answer at all.

### Recommended record-the-modes-you-care-about workflow

For a daily-driver user:

```sh
# One recording per mode the user actually uses:
zshrs-recorder --tag interactive,login                    # default invocation
zshrs-recorder --tag non-interactive --command 'source ~/.zshrc'
zshrs-recorder --tag remote-ssh --env SSH_CONNECTION=fake
zshrs-recorder --tag dumb-term --env TERM=dumb
```

The daemon stores 4 runs, each tagged. `zwhere` queries default to
the most recent run matching the current shell's mode (looked up
from `$TERM`, `$-`, `$SSH_CONNECTION`, `$0` at query time), or can
explicitly target any run. Mode-tagged caching is a feature, not a
bug — it lets users compare their shell's behavior across modes
without ever leaving a query.

### The patent-claim addition

In addition to plugin-framework-agnosticism, this gives the recorder
a SECOND structural moat: **every mode-quirk that has historically
required per-framework, per-mode adapter code in shell introspection
tools (interactive vs non-interactive sourcing chains, login vs
non-login bootstrap, sandbox/restricted-mode conditional loads, TERM
variation, KSH/SH emulation modes) is dissolved by deferring all
conditional resolution to the runtime and observing the resulting
state mutations.** No tool that statically analyzes config files can
match this because the input space (runtime conditional branches)
exceeds the analysis space (parse-time predicates).

This is provably impossible to retrofit onto static-analysis tools
without re-architecting them into runtime tools — at which point
they ARE the recorder.

### The patent-claim phrasing

The novel capability is a **runtime-AOP intercept layer at the
union-of-state-mutating-builtin-dispatchers, providing a structurally
universal substrate for plugin-manager-agnostic shell-state attribution
that is forward-compatible with arbitrary future plugin frameworks
without code changes to the recorder.**

That phrasing matters because:
- "runtime AOP" → distinct from static analysis (zinit-report) and
  text tracing (set -x)
- "union-of-...-dispatchers" → exhaustive coverage of state mutations,
  not a subset
- "structurally universal substrate" → forward-compat is *guaranteed*
  by the architecture, not by recurring engineering effort
- "without code changes to the recorder" → unique selling point;
  contrast every existing tool, which requires per-framework
  adapter code

This is the structural moat. zinit-report cannot ever catch up
without re-architecting around runtime intercepts; doing so would
make zinit-report into the recorder. Every other framework has the
same constraint. The recorder is a one-way ratchet: once shipped,
no tool can match its coverage without copying its architecture.

## The AOP framing

The intercept layer is **aspect-oriented**: the cross-cutting concern
of "record this state mutation" is woven into every builtin
dispatcher without modifying the dispatcher's logic. zshrs's runtime
already supports this via the AOP framework cited in the patent
strategy (claim B / claim C, `aot_patent_strategy` memory).

Mechanics of an aspect (one per state-mutating builtin):

```
Before(alias_dispatch):                                      [1]
  capture (file, line, fn_chain, ts)                         [2]
  emit_record(kind="alias", name=$1, value=$2, ...)          [3]
Around(alias_dispatch):                                      [4]
  proceed_with_original_logic()                              [5]
After(alias_dispatch):                                       [6]
  if exit_status != 0: mark record as failed_apply           [7]
```

This is structurally novel for a Unix shell. No existing shell has
runtime aspects woven across builtin dispatch. bash's `set -x` is the
closest comparable feature and it dumps unstructured text to stderr;
it cannot answer queries, doesn't persist across shells, and has no
schema. The recorder treats every builtin invocation as a *join
point* and the metadata emission as an *advice*. Standard AOP
vocabulary applied to a shell runtime for the first time.

### Why "runtime AOP" and not "wrap each function"

The naive alternative is: shadow every builtin with a shell function:

```zsh
alias() {
    ZSHRS_RECORD alias "$@"
    builtin alias "$@"
}
```

This breaks for several reasons:

1. **Recursion.** `alias` is called from oh-my-zsh's helper functions, which would re-enter the shadow. Adding guard variables fixes per-call but slows everything.
2. **Argument-list fidelity.** Shell quoting + `$@` expansion + reserved-word handling differs subtly between `builtin alias`, `alias` shadow, and the original. Edge cases (e.g. `alias -gs` for suffix aliases, `alias -L` for listing) require per-form forwarding.
3. **Position context loss.** By the time the shadow function runs, `$LINENO` reports the line of the shadow itself, not the caller's `alias` invocation. You'd need `funcfiletrace[1]` parsing to recover the caller — fragile and slow.
4. **Coverage gaps.** `typeset` from `local` has different syntax; assignments via `VAR=value` (an *assignment* token, not a builtin call) bypass any function shadow entirely. Pure-shell instrumentation fundamentally cannot see assignment-syntax mutations.
5. **Performance.** Every builtin call now allocates a function frame, consults the alias table, runs argv-rewrite logic. Estimates: 5-50× slowdown on `compinit`, which makes ~20k builtin calls.

Runtime AOP at the dispatcher level avoids all five. The aspect runs *inside the lexer's already-parsed call site*, with full access to the AST node (lineno, filename), the original argv (untouched), and the runtime's funcstack. No recursion, no quoting hazard, no shadow function overhead.

## Architecture

### Components

```
                ┌──────────────────────┐
                │   .zshrc / ZDOTDIR   │
                └──────────┬───────────┘
                           │
                  zshrs-recorder
                           │
                  ┌────────▼─────────┐
                  │  AOP intercept   │   ← weaves before-advice on every
                  │  on dispatchers  │      state-mutating builtin
                  └────────┬─────────┘
                           │ batched RecordEvent
                           │ (unix socket, daemon.sock)
                           ▼
                  ┌─────────────────────────┐
                  │   zshrs-daemon          │
                  │ ┌────────────────────┐  │
                  │ │ catalog (SQLite)   │  │  ← canonical, queryable
                  │ │  definitions table │  │
                  │ ├────────────────────┤  │
                  │ │ rkyv shard         │  │  ← mmap-fast read path
                  │ │  records-{run}.rkyv│  │     for thin clients
                  │ └────────────────────┘  │
                  └────────────┬────────────┘
                               │
                               ▼
                  ┌─────────────────────────┐
                  │   zshrs (any shell)     │
                  │  zwhere alias gst       │  ← <50µs lookup
                  │  zwhere file ~/.zshrc   │
                  │  zwhere when path       │
                  └─────────────────────────┘
```

### Two binaries, three usage modes

The recorder ships as a SEPARATE binary, `zshrs-recorder`, built from
the same source tree under a Cargo feature flag. The default `zshrs`
binary is compiled WITHOUT the feature; recorder code is removed by
the compiler before linking.

| Binary | Built with | Purpose |
|---|---|---|
| `zshrs` | `cargo build` (default features, **no** `recorder` feature) | Daily-driver. Read path only. **Contains zero bytes of recorder code** — verified by `objdump`/`nm` post-build. Looks up records that the recorder previously persisted via daemon for `zwhere` queries. |
| `zshrs-recorder` | `cargo build --bin zshrs-recorder --features recorder` | One-shot indexing. Source the user's init, capture every state mutation via AOP aspects, batch-emit to daemon, exit. Used for first-time setup or after major config changes. |

Three usage modes from the user's perspective:

| Invocation | Resolves to | Purpose |
|---|---|---|
| `zshrs-recorder` | new binary | Manual one-shot recording run. |
| `zshrs` | default binary | Daily-driver. `zwhere` queries are read-only against the daemon's catalog. |
| daemon `--watch` | daemon background trigger | Daemon's fsnotify on configured paths spawns `zshrs-recorder` automatically when a sourced file changes. |

The two binaries share the entire codebase except the recorder
aspects, IPC types, and query-side code, all of which live behind
`#[cfg(feature = "recorder")]`. There is exactly one source of truth
for the runtime; the binaries diverge only in which `cfg`-gated code
the compiler keeps.

## Zero-overhead default binary (non-functional requirement #1)

**The default `zshrs` binary contains zero recorder code.** Not "zero
overhead at runtime" — *zero bytes*. Daily-driver users running
`zshrs` get a binary that is byte-identical to one built before the
recorder feature ever existed. The recorder lives entirely behind a
Cargo feature flag, deleted by `rustc` during compilation when the
flag is off, so there is no possibility of:

- a misconfigured runtime accidentally enabling recording
- a runtime branch checking a flag before every builtin
- an indirect dispatch pointer adding even a BTB cycle
- an extra field in `ZshCommand` / `ZshFuncDef` etc. for recorder bookkeeping
- supply-chain risk from recorder-side IPC code being part of the daily-driver attack surface

The mechanism is conditional compilation, not runtime indirection.

### Cargo configuration

```toml
# Cargo.toml (root)
[features]
default = []
recorder = ["dep:rusqlite", "dep:rkyv"]   # already deps in workspace, just lifted
                                          # behind feature for `zshrs` default binary

[[bin]]
name = "zshrs"
path = "bins/zshrs.rs"
# default features only — `recorder` is OFF

[[bin]]
name = "zshrs-recorder"
path = "bins/zshrs-recorder.rs"
required-features = ["recorder"]          # cargo refuses to build this bin
                                          # without the feature
```

`cargo build`           → emits `zshrs` (no recorder).  
`cargo build --features recorder --bin zshrs-recorder` → emits `zshrs-recorder`.  
`cargo build --features recorder --bins` → emits both.

The release CI matrix builds:

| Job | Command | Output |
|---|---|---|
| `default` | `cargo build --release` | `target/release/zshrs` (no recorder) |
| `with-recorder` | `cargo build --release --features recorder --bin zshrs-recorder` | `target/release/zshrs-recorder` (with recorder) |

### Three implementation rules enforce zero-default-overhead

#### Rule 1 — Every recorder symbol is `#[cfg(feature = "recorder")]`

The recorder module:

```rust
// src/recorder/mod.rs — entire module gated
#![cfg(feature = "recorder")]

pub mod aspects;
pub mod ipc;
pub mod buffer;

pub fn install_aspects(runtime: &mut Runtime) { ... }
pub fn flush_on_exit() { ... }
```

The aspect code:

```rust
// src/exec/builtins/alias.rs
pub fn bin_alias(args: &[&str]) -> i32 {
    do_alias(args)              // ← only this in default build
}

#[cfg(feature = "recorder")]
pub fn bin_alias_recording(args: &[&str], ctx: RecordCtx) -> i32 {
    crate::recorder::aspects::emit_alias_event(args, ctx);
    do_alias(args)
}
```

The runtime registration:

```rust
fn build_runtime(opts: &Opts) -> Runtime {
    let mut rt = Runtime::default();
    register_builtins(&mut rt);

    #[cfg(feature = "recorder")]
    if opts.record {
        crate::recorder::install_aspects(&mut rt);  // swaps dispatchers
    }

    rt
}
```

In a default build, `crate::recorder` does not exist as a name in scope. The `#[cfg]`-gated `install_aspects` block is removed by the parser/expansion phase. The default `zshrs` binary contains no `recorder::*` symbols.

#### Rule 2 — Recorder dependencies are feature-gated too

`Cargo.toml`'s `[dependencies]` declares recorder-only deps as optional and pulls them in only via the `recorder` feature:

```toml
[dependencies]
# ... shared deps (rusqlite, rkyv) are already used by daemon, but for
# the recorder-IPC-emit-only code path we keep them out of zshrs's
# default build by re-routing through the `recorder` feature on the
# zshrs side. The daemon crate uses them unconditionally.
rusqlite = { version = "0.32", optional = true }
rkyv     = { version = "0.7", optional = true, features = [...] }

[features]
default = []
recorder = ["dep:rusqlite", "dep:rkyv"]
```
#### Rule 3 — `zwhere` is always in default `zshrs`

The recorder feature gates the **AOP write path** (the runtime
intercepts that emit records during shell init). The **query path**
is just an IPC client over `definitions_query` — same tier as
`zcache`, `zls`, `zping`. zshrs already links the daemon-client
crate, so the marginal cost of shipping `zwhere` in the default
binary is a few hundred bytes of code, not a separate feature.

`zwhere` lives in `daemon/builtins.rs` alongside the other z*
builtins:

```
zwhere KIND [NAME]              # rows for KIND [NAME] across all shells
zwhere KIND --prefix STR        # all KIND rows whose name starts with STR
zwhere --kinds                  # list every populated kind
zwhere --shell-id ID …          # restrict to one shell_id
zwhere --limit N …              # cap (default 1000)
```

Output: tab-separated `kind\tname\tvalue\tshell_id\tfile:line`,
script-friendly. Pipe through `column -t` for human reading.

The earlier draft of this doc claimed `zwhere` would be feature-gated;
that was incorrect reasoning. Read-only query has no dependency on
the recorder's write-path code, so there's nothing to gate.

### Validation

The zero-recorder-code claim is empirically verifiable, not just asserted:

| Test | Command | Pass criterion |
|---|---|---|
| **Symbol absence** | `nm target/release/zshrs \| grep -c recorder` | `0` |
| **String absence** | `strings target/release/zshrs \| grep -c -i recorder` | `0` (modulo benign matches in third-party deps unrelated to our recorder; whitelist check) |
| **Disassembly absence** | `objdump -d target/release/zshrs \| grep -c "recorder::"` | `0` |
| **Binary-size delta** | `wc -c target/release/zshrs` before vs after recorder feature added | `≤ +0 bytes` |
| **CI gate** | All four checks above run on every release tag | Block merge on regression |

| Performance benchmark | Baseline (pre-recorder) | Target (post-recorder, default build) |
|---|---|---|
| Cold-start `zshrs -c 'echo hi'` | T₀ | T₀ ± noise (no measurable change) |
| Sourcing zpwr's init | T₁ | T₁ ± noise |
| `compinit` (compsys cold) | T₂ | T₂ ± noise |

There is no perf regression possible because the default binary literally contains no recorder code. The benchmarks exist to catch accidental introduction of a non-`#[cfg]`-gated branch — a regression test, not a tolerance budget.

If `nm | grep recorder` ever returns nonzero in the default release artifact, that's a release blocker.

## Interception surface

The complete list of state-mutating builtins to intercept (one aspect per row):

| Kind | Builtin / syntax | What's recorded |
|---|---|---|
| `alias` | `alias name=value` | name, value, type=normal/global/suffix |
| `function` | `name() { ... }` and `function name { ... }` and `autoload name` | name, body (or autoload-only flag), funcfile if known |
| `assign` | `VAR=value` (top-level assignment, not local) | name, value, prev_value (if any) |
| `typeset` | `typeset` / `declare` / `readonly` / `integer` / `float` / `local` | name, type flags, value, scope |
| `export` | `export NAME[=value]` | name, value |
| `path_mod` | `path+=...`, `fpath+=...`, `manpath=...`, `module_path+=...` | array name, ops (added/removed/reordered), final state hash |
| `hash_d` | `hash -d name=path` | name, path |
| `zstyle` | `zstyle pattern style values...` | pattern, style, values |
| `bindkey` | `bindkey [-M map] sequence widget` | keymap, sequence, widget |
| `compdef` | `compdef _func cmd ...` | function-name, command list |
| `zmodload` | `zmodload mod` (loading), `zmodload -F mod +/-feat` | module, features |
| `setopt` | `setopt OPT` / `unsetopt OPT` / `set -o opt` | option name, on/off |
| `trap` | `trap 'cmd' SIG` | signal, handler |
| `sched` | `sched +M:S cmd` | absolute time, command |
| `source` | `source FILE` / `. FILE` | resolved absolute path, mtime, inode (already in daemon source_resolver) |

**~15 surfaces.** Bounded by zsh's grammar; new builtins in upstream zsh are rare events (last decade: maybe 3 additions). Each surface is one dispatcher in zshrs's runtime; the aspect is mechanical.

### What's NOT intercepted (and why)

- **Pure-execution builtins.** `echo`, `printf`, `cd`, `pushd`, `popd`, `eval` (the eval result IS intercepted via the eval'd code re-entering the lexer; the eval call itself is not a state mutation).
- **Read-only introspection.** `whence`, `which`, `type`, `print`, `getopts`.
- **Job control.** `bg`, `fg`, `jobs`, `wait` — runtime state, not config.
- **External commands.** Cannot intercept; they run in subprocesses.
- **Reading from stdin.** `read VAR` does mutate VAR; included under `assign` if the variable is then used by config, but typically interactive (recorder skips by default — opt-in via `--record-reads`).

## Schema

### SQLite (canonical)

```sql
-- One row per recorder run.
CREATE TABLE runs (
    run_id           INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at_ns    INTEGER NOT NULL,
    finished_at_ns   INTEGER,
    cmdline          TEXT,           -- 'zshrs-recorder'
    zdotdir          TEXT,           -- snapshot of $ZDOTDIR
    home             TEXT,
    record_count     INTEGER,
    notes            TEXT
);

-- One row per state-modifying event.
CREATE TABLE definitions (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id           INTEGER NOT NULL REFERENCES runs(run_id),
    order_idx        INTEGER NOT NULL,    -- sequence within run
    ts_ns            INTEGER NOT NULL,
    kind             TEXT    NOT NULL,    -- 'alias', 'function', 'export', ...
    name             TEXT    NOT NULL,    -- 'gst', 'PATH', '_git', ...
    value            TEXT,                -- alias body, fn source (truncated >4KB → hash + blob ref)
    value_hash       BLOB,                -- sha256 if truncated
    file             TEXT,                -- absolute path; NULL if synthesized (e.g. eval)
    line             INTEGER,
    col              INTEGER,
    fn_chain         TEXT,                -- 'compinit > _comp_setup > zpwrLoadAliases'
    flags            INTEGER NOT NULL DEFAULT 0,  -- bitmask: dynamic_eval, sourced, plugin_loaded, ...
    prev_value       TEXT,                -- previous value if this overwrote one
    prev_def_id      INTEGER REFERENCES definitions(id),  -- earlier definition this shadowed
    sensitive        INTEGER NOT NULL DEFAULT 0   -- secret-bearing flag (PATH != sensitive; AWS_SECRET_ACCESS_KEY = sensitive)
);
CREATE INDEX idx_definitions_kind_name ON definitions(kind, name);
CREATE INDEX idx_definitions_file_line ON definitions(file, line);
CREATE INDEX idx_definitions_run ON definitions(run_id, order_idx);

-- Per-name index of latest definition (denormalized cache for fast `zwhere` lookup).
CREATE TABLE current_defs (
    kind             TEXT NOT NULL,
    name             TEXT NOT NULL,
    def_id           INTEGER NOT NULL REFERENCES definitions(id),
    run_id           INTEGER NOT NULL,
    PRIMARY KEY (kind, name)
);
```

### rkyv shard (read path)

`~/.zshrs/recorder-{run_id}.rkyv`: serialized
`Vec<DefinitionRecord>` with byte-aligned layout suitable for `mmap`
+ zero-copy access. Each record is the same data as a SQLite row but
in a fixed-shape struct.

```rust
#[derive(rkyv::Archive, rkyv::Serialize)]
pub struct DefinitionRecord {
    pub kind: u8,                    // enum DefKind (compact)
    pub name: rkyv::string::ArchivedString,
    pub value: Option<rkyv::string::ArchivedString>,
    pub file_idx: u32,                // index into per-shard file table
    pub line: u32,
    pub col: u16,
    pub fn_chain_idx: u32,            // index into chain table
    pub flags: u16,
    pub order_idx: u32,
    pub ts_ns: u64,
}
```

The thin client (regular `zshrs`) doesn't talk to SQLite for hot
lookups — it mmaps the rkyv shard via the daemon's catalog hint and
binary-searches by `(kind, name)`. Same pattern as zshrs's existing
autoload cache (`docs/DAEMON.md`). Lookup latency: 50ns to 5µs
depending on cold/warm cache.

## IPC and batching

Per-event IPC over the daemon socket would tank `compinit` (~20k
events). Batching:

```
Recorder buffer: VecDeque<RecordEvent>, soft cap N=512 events
                                        hard cap M=4096 events

Flush triggers:
  - buffer.len() >= N
  - 50ms since last flush AND buffer.len() > 0
  - phase boundary (e.g. precmd hook fires, indicating init done)
  - shell exit
```

Single batch flush = one SOCK_STREAM write of a length-prefixed
serialized `Vec<RecordEvent>`. Daemon side: single SQLite
transaction, `PRAGMA synchronous=NORMAL`, ~10ms for 4096 inserts.

Total recorder run overhead estimate (zpwr + zinit + p10k): **~1.5×
normal startup time**, dominated by SQLite IO. Acceptable for a
one-shot indexing run; not suitable for daily driver (which is why
recorder is opt-in).

## End-of-run autoload prewarm

The last thing a recording does, after the init chain has finished and
before the bundle ships, is compile every `_*` completer on the recorded
`$fpath` into `~/.zshrs/autoloads.rkyv` (`--no-prewarm` opts out).

This is the same work the shell would otherwise do lazily, one completer
at a time, at a prompt: the loader installs an autoloaded function by
running its definition program, and caches that compiled chunk keyed by
name + the file's mtime and length. Front-loading it means the first
`ls -<TAB>` after a fresh install is an O(1) probe into the shard rather
than a parse + compile of the completer's file — for `_git`, 229 µs of
chunk decode instead of 318 ms of parse + compile.

**Why here and not in `compinit`.** `parse()` walks process-global lexer
state. An earlier version of this ran on `compinit`'s worker pool
concurrently with the interactive main thread and corrupted it — the
prompt ended up emitting the xtrace prefix and stuck in PS2, so the pass
was disabled behind an env var and its output (a bare file body compiled
as a top-level script) was never read by anything. The recorder is a
one-shot process that never returns to a prompt, which makes it the only
place this can run safely today. The same pass is reachable as
`zshrs --prewarm-autoloads` and, through the daemon, as `zd prewarm`.

**Cost.** Roughly 6× the source size in bytecode and ~0.84 ms per
completer (debug build): a 13k-file directory took 35 s and produced
165 MB. Entries already current are skipped by mtime + length, so a
re-run after installing one plugin is one `stat` per completer.

## fn_chain capture

The function call-stack at the time of definition is captured from
zsh's existing `funcstack` array (per CLAUDE.md memory: zshrs
already plumbs this through the parser via `ZshPipe.lineno`). The
implementation reads the current funcstack and joins names with `>`:

```
funcstack: ['compinit', '_comp_setup', 'zpwrLoadAliases']
           ↓
fn_chain: "compinit > _comp_setup > zpwrLoadAliases"
```

For top-level (sourced from `.zshrc` directly, no enclosing
function), `fn_chain` is empty and `file`/`line` alone identify the
site.

For `eval`-generated code:

```
eval "alias x=y"   ← in ~/.zshrc:42
```

We record `file=~/.zshrc`, `line=42`, `flags |= DYNAMIC_EVAL`,
`fn_chain` reflects the funcstack at the eval call site. Users can
filter with `zwhere -d` (dynamic-only) or `zwhere -D` (exclude
dynamic).

## Query API

A new builtin `zwhere`:

```
USAGE
   zwhere KIND NAME              # exact match within KIND
   zwhere all NAME               # match across all kinds
   zwhere -f FILE                # everything defined in FILE
   zwhere -F PATTERN             # files matching glob PATTERN
   zwhere -k KIND                # all entries of given kind
   zwhere -t SINCE               # entries since timestamp / run id
   zwhere -d / -D                # only dynamic-eval / exclude dynamic
   zwhere -l NAME                # full lineage (every prior def, in order)
   zwhere -o KIND NAME           # `--origin` — first definition only

OUTPUT
   /Users/wizard/.zpwr/env/.shell_aliases_functions.sh:1742:42 (zpwrLoadAliases ← _zpwr_init)
       gst → 'git status -sb'

   With -l (lineage):
   3 definitions of `gst`:
   1. /Users/wizard/.zinit/snippets/OMZ::lib/git.zsh/OMZL::git.zsh:188   gst → 'git status'  [overridden]
   2. /Users/wizard/.zpwr/env/.shell_aliases_functions.sh:1742           gst → 'git status -sb'  [overridden]
   3. /Users/wizard/.zshrc:567                                            gst → 'git status -sb --branch'  [current]
```

Output format is two-line: location header + content body. Stable
columns for grep-ability:

```
zwhere -k alias | grep ' git '
```

### Timing-based queries

Beyond "where", "WHEN" matters for debugging load order:

```
zwhere when path                  # every site that mutated $path, in order
zwhere when fpath                 # every site that touched $fpath
zwhere when -k setopt PROMPT_SUBST   # when did PROMPT_SUBST get enabled
```

Output uses `order_idx` to reconstruct the sequence:

```
1.  ~/.zshenv:5         path=(/usr/local/bin $path)
2.  ~/.zshrc:21         path+=(~/.local/bin)
3.  zinit_init:101      path=(/opt/homebrew/bin $path)            [via zinit ice]
4.  ~/.zpwr/env/.zpwr_env.sh:88   path+=($ZPWR/scripts)
5.  /etc/zshrc:12       path+=(/usr/sbin)                         [SYSTEM]
```

That's a debugging superpower zsh users do not have today.

## User-facing impact: the daily-driver workflow change

This is not just a power-user feature. It changes a basic
debugging operation that every zsh user with a non-trivial config
has hit and lived without an answer to.

### The grep-the-haystack workflow today

Typical zsh user with a real plugin setup:

| Stack component | Approximate LOC / file count |
|---|---|
| `~/.zshrc` | 100-2,000 lines |
| oh-my-zsh (if installed) | ~200 plugin dirs, ~50k LOC |
| zinit + plugins | 5-50 plugins, 10k-200k LOC |
| zsh-more-completions / completion plugins | 700 to 27,387 files |
| Custom `~/.zsh_*` files | varies widely |
| Framework-specific env files (zpwr, prezto, etc.) | 10k-200k LOC |
| **Combined search surface** | **typically 50k-300k+ LOC of zsh code** |

When the user wonders "where did this alias come from?", the
existing options are:

```sh
$ whence -v gst
gst is an alias for git status -sb
# ↑ tells you the value, not the source
```

```sh
$ which -a gst
gst: aliased to git status -sb
# ↑ same
```

```sh
$ grep -rn "alias gst" ~/.zshrc ~/.oh-my-zsh ~/.zinit ~/.zpwr 2>/dev/null
~/.oh-my-zsh/plugins/git/git.plugin.zsh:181:alias gst='git status'
~/.zpwr/env/.shell_aliases_functions.sh:1742:alias gst='git status -sb'
~/.zinit/plugins/MenkeTechnologies---zsh-z/...:88:# alias gst='git status'  # commented out
~/.zsh_local:23:alias gst='git status -sb --branch'
~/.zinit/snippets/OMZL::git.zsh/...:188:alias gst='git status'
# ↑ five matches across four files. Which one actually fired?
#   What load order? Did any get skipped due to conditional guards?
#   Was one defined inside a function that wasn't called?
#   The user has to manually trace each.
```

For functions it's worse — the function might be loaded via
`autoload`, defined in a plugin, wrapped by another plugin,
overridden by `.zshrc`, with the actual call site buried under 3-5
levels of plugin-manager indirection.

### Time cost (typical zsh user with real config)

| Operation | Today | With recorder |
|---|---|---|
| "Where is alias `gst` defined?" | 30s-5min (grep + inspect) | <100µs (`zwhere alias gst`) |
| "Where did `_my_completion` function come from?" | 5-30min (autoload chain trace) | <100µs |
| "Why is `PATH` showing `/opt/...` first?" | 5-60min (every PATH= site traced manually) | `zwhere when path` shows the ordered list |
| "Which plugin enabled `EXTENDED_GLOB`?" | 10-30min (every `setopt` site grep) | `zwhere when -k setopt EXTENDED_GLOB` |
| "What aliases did the `git` plugin define?" | 5-15min | `zwhere -F '*git*'` |
| "What changed between my interactive and SSH sessions?" | impossible-without-side-by-side-diff | `zwhere -m diff interactive remote-ssh` |

For a user actively configuring their shell — which is most zsh
power users — these operations come up multiple times per week.
The cumulative time savings are hours per month per user.

### The "first time ever in shell history" claim

Every existing introspection tool has been a partial solution:

- `whence` / `which` / `type`: gives you the value, never the
  source location
- `functions -t name`: runtime trace of one already-called
  function; useless for un-called functions
- `set -x` / `xtrace`: voluminous unstructured stderr; you have to
  manually search through thousands of lines of trace output
- `zinit @zinit-report`: only zinit-loaded plugins, only
  per-plugin granularity, no file:line
- `compdump` inspection: only completions, no aliases / functions /
  variables
- `grep` over plugin dirs: confounded by conditionals, comments,
  override order, dynamic loading

**The recorder is the first time any shell user can ask "where is
the exact line where this thing was defined?" and get a definitive,
instant, structured answer for any state mutation across their
entire shell config**, regardless of:

- which plugin manager loaded it (or whether a plugin manager
  was used at all)
- whether the definition was conditional, deferred, or generated
  via `eval`
- whether it was defined in interactive, non-interactive, login,
  or SSH mode
- whether it was overridden by a later definition
- how many transitive `source` chains preceded the definition

This is not a new tool added to the existing toolbox. It's a
capability that did not exist in any Unix shell before, made
possible by zshrs's global AOP layer (per "Why this can only exist
in zshrs" above), and surfacing through a query (`zwhere`) that's
faster than the user could even think to type the file path
they're looking for.

### Adoption-friction analysis

For any zsh user already running zshrs, the upgrade path is:

```sh
zshrs-recorder         # one-time, ~1.5x normal startup duration
# ... daily-driver continues using `zshrs` normally; zero overhead ...
zwhere alias gst       # instant
```

No config file changes. No plugin-manager-specific setup. No
manual annotation of plugins. The recorder discovers the user's
state by RUNNING their existing config — whatever that config
looks like, whoever wrote it, whatever framework wrote it. There
is no migration cost.

For users not yet on zshrs: this is one of the strongest
"reasons to switch" the project can offer, alongside
parallel-completion-load and AOT-compiled startup. Faster startup
is incremental; this is qualitatively new capability.

## Shell environments as portable, version-controlled, distributable artifacts

Once the recorder produces a complete end-state artifact (the rkyv shard
+ definitions table for one run), the shell environment itself becomes a
file that can be treated like any other build output. This is the
second-order benefit: not just faster startup, but a structural shift
in how shell environments are shipped, versioned, audited, and
reproduced.

### What today's shell-config-as-source-tree model cannot do

Today, "my shell config" is the union of:

- `~/.zshrc`, `~/.zshenv`, `~/.zprofile`, `~/.zlogin`, `$ZDOTDIR/*`
- Plugin manager state (`~/.zinit/plugins/*` clones, `~/.oh-my-zsh/`
  install dir, `fisher`'s `fish_plugins` file)
- `.zcompdump*` (corruptible)
- `~/.cache/p10k-*`, `~/.zinit/abbrevs/*`, etc.
- Possibly `/etc/zshrc`, `/etc/zprofile` from system packaging
- Universal env vars (`fish_variables` for fish; not present in zsh)
- Implicit dependencies on `$PATH` order, installed binaries, FS layout

This is not a coherent artifact. It cannot be:

- Shipped to another machine deterministically — clone the repo and
  run zinit update, then hope versions align
- Reproduced exactly — plugin manager pulls latest commits, `.zcompdump`
  may corrupt, conditional load branches differ across machines
- Diffed at the runtime level — `git diff ~/.zshrc` shows source
  changes; says nothing about whether the resulting shell behaves
  differently
- Rolled back atomically — comment out plugins, restart, manually
  bisect, repeat
- Published as a unit — there is no unit
- Signed for supply-chain attestation — there is nothing concrete to
  sign

### What the recorder's snapshot artifact unlocks

The complete end-state for one recorder run is a small set of files
(per `docs/DAEMON.md`'s sharded layout):

- `~/.zshrs/recorder-{run_id}.rkyv` — the rkyv shard with
  every definition record, mmap-ready
- `~/.zshrs/catalog.sqlite` — the definitions table queryable
  for inspection
- `~/.zshrs/manifest-{run_id}.json` — header (modes, source
  files, timestamps, hash chain)

Together: typically 1-10 MB for even an extreme power-user config
(zpwr + zsh-more-completions + zinit + 50 plugins). One artifact;
one byte-checksum-able blob.

### Capability table

| Capability | Source-tree model (today) | Snapshot artifact model (recorder) |
|---|---|---|
| Ship to another machine | rsync `~/.zshrc` + clone every plugin repo + run `zinit update` + hope | `scp recorder-{id}.rkyv user@host:~/.zshrs/`; `zshrs` next launch on host has byte-identical shell |
| Reproducible across machines | broken: plugin version drift, missing dependencies, `.zcompdump` corruption, different fpath, conditional loads | guaranteed: same shard → same state, deterministically |
| Version control diffs | `git diff ~/.zshrc` shows source changes; says nothing about runtime result | `git diff snap-A.rkyv snap-B.rkyv` (via deserialized form) shows every state change between snapshots |
| Atomic rollback | manually comment out plugins, restart, debug, repeat | `zwhere snapshot restore <run_id>` swaps the active shard; instant rollback |
| Branch / experiment | duplicate dotfiles + plugin manager state, manual switch | symlink active shard to a different `recorder-{id}.rkyv` |
| Audit (security / compliance) | trace through dotfiles → plugin manager → maybe-runs maybe-doesn't | `SELECT * FROM definitions WHERE run_id = ?` — every alias/function/binding visible |
| Corporate "blessed" environment | dotfile bootstrap script that "works on most machines" | publish `blessed-shell-v2026-Q1.rkyv` to artifact registry; every employee pulls + loads → same shell |
| Per-project shells | per-repo `.envrc` + direnv + ad-hoc plugin loads | per-repo `.zshrc-snapshot.rkyv`; daemon swaps on `cd` (per `docs/DAEMON.md` source-resolver layer) |
| Time-travel | "what did my shell look like 6 months ago?" — undefined; no snapshot exists | load 6-month-old shard, diff against current |
| Bisect a regression | manually comment out plugins until it works | `zwhere snapshot bisect <good_run_id> <bad_run_id>` — binary-search to the offending definition |
| Ship to CI | install zsh + zinit + plugins + warm `.zcompdump` in CI image — minutes per CI job | drop `recorder-{id}.rkyv` into image; CI shell starts in 50ms vs 5s |
| Sign for supply-chain | no unit to sign | `zwhere snapshot sign <key>` — sign the shard manifest for trust attestation |

### The Docker analogy is exact

Pre-Docker: deployment was a tree of source + install scripts. Same
source produced different states across machines because of
environment drift, package version drift, conditional logic. "Works on
my machine" was the universal bug class.

Post-Docker: deployment is a byte-identical image. The runtime is
loading a snapshot, not rebuilding from source. Reproducibility
becomes free; "works on my machine" becomes "works because the image
is byte-identical."

The shell ecosystem has been **pre-Docker for 50 years**. Every other
deployment domain has the snapshot artifact primitive (Docker images,
VM images, AMIs, Nix store paths, OCI artifacts, IDE workspace
exports). Shells uniquely lack it. The recorder's rkyv shard is the
shell-config equivalent of an OCI image — addressable, reproducible,
distributable, signable.

### Tooling enabled by the snapshot artifact

The recorder ships the build phase. Shipping the full snapshot-artifact
toolchain on top is straightforward, since the shard format is already
designed for distribution:

| Subcommand | Purpose | Implementation cost |
|---|---|---|
| `zwhere snapshot save [--tag NAME]` | Mark current shard as a named snapshot; copy to `~/.local/share/zshrs/snapshots/` | trivial; copy + manifest update |
| `zwhere snapshot list` | List all named snapshots with metadata (date, modes, def count, signed-by) | sqlite SELECT |
| `zwhere snapshot load NAME` | Swap the daemon's active shard atomically | one IPC op + fsync + rkyv mmap re-bind |
| `zwhere snapshot diff A B` | Show definitions present in A but not B, vice versa, value changes | join the two `definitions` tables on `(kind, name)` |
| `zwhere snapshot bisect GOOD BAD` | Binary-search the run-order between two snapshots to find first regression | sqlite range queries on `order_idx` |
| `zwhere snapshot publish [--registry URL]` | Push shard + manifest to a registry (S3, OCI, GitHub releases) | http PUT + content-addressing |
| `zwhere snapshot pull URL` | Fetch + verify + load a published shard | http GET + signature check + load |
| `zwhere snapshot sign --key PATH` | Sign manifest with a private key (sigstore-compat) | standard ed25519 + cosign-format manifest |
| `zwhere snapshot verify --pubkey PATH` | Verify signature against trusted public key | inverse of sign |
| `zwhere snapshot freeze` | Mark current shard as immutable; subsequent recorder runs go to a new shard | sqlite flag + permission bit |

This is a couple weeks of work on top of Phase 1-5 (the recorder
itself). Ships as Phase 6.

### Distribution scenario: zpwr as a published shell artifact

Concretely for the zpwr ecosystem:

```sh
# At zpwr release time (run once on the maintainer's machine):
zshrs-recorder --tag zpwr-v48.7.3 --env ZPWR_REMOTE=false
zwhere snapshot save --tag zpwr-v48.7.3
zwhere snapshot sign --key ~/.zshrs/release.key
zwhere snapshot publish --registry github://MenkeTechnologies/zpwr

# A user who wants zpwr (instead of cloning + running install.sh):
zwhere snapshot pull github://MenkeTechnologies/zpwr:v48.7.3
zwhere snapshot load zpwr-v48.7.3
# done — every alias, function, binding, completion is loaded; cold
# shell starts in 50ms instead of 3-5s; no zinit, no install.sh, no
# clone needed
```

zpwr distribution stops being "clone the repo + run install + hope"
and becomes "pull the artifact + load." This is the same shift Docker
made for OS deployment, scoped to the shell layer. zpwr (172k LOC, 506
subcommands, 2k aliases, 17k completions) compresses to a sub-10MB
shard that any user can pull and load in under a second.

### Patent-claim addition

A third dependent claim under the global-AOP foundation:

> Method for capturing the complete runtime state of a Unix shell
> following init-script execution, serializing said state to a
> portable, content-addressable, signable artifact, and rehydrating
> said state into a separate shell process via memory-mapped
> deserialization, enabling deterministic reproduction, atomic
> rollback, version control, and registry-based distribution of shell
> environments.

The novelty surface includes:

- **Portable artifact format** — rkyv shard + manifest; content-addressed;
  signable for supply-chain attestation
- **Deterministic reproduction across machines** — same shard produces
  byte-identical shell state, eliminating dotfile drift / plugin version
  drift / `.zcompdump` corruption
- **Registry-based distribution** — shells become OCI-image-like artifacts
  pushable/pullable from registries
- **Diff/bisect/rollback over runtime state** — operations available on
  filesystem trees today are extended to shell environments

No existing shell tool produces a portable runtime-state artifact —
zinit `@zinit-report` is a text report (not loadable), fish's
universal-variables file is one component (not the full state),
`.zcompdump` is a single subsystem's cache (corruption-prone, not
distributable). The shard-as-artifact pattern is novel for the shell
category.

## What this beats

| Tool | What it shows | What's missing |
|---|---|---|
| `whence -v gst` | "gst is an alias for git status" | source location |
| `which -a gst` | function body | source location, alias coverage |
| `functions -t name` | runtime trace of one function call | metadata for un-called functions, scope across defs |
| `zinit @zinit-report` | zinit-loaded plugins, ICE state | non-zinit code, file:line, override chain |
| `omz reload` / `omz changelog` | OMZ plugin info | non-OMZ code, file:line |
| `set -x` / `xtrace` + `PS4` | execution trace stderr | structured query, persistence, schema |
| `compaudit` | completion-dir security | non-completion definitions |
| `zsh -i -c 'set -o' \| diff` | option diff between configs | file:line for which `setopt` line set what |
| `zsh-newuser-install` | none | everything |

The recorder fills the union of all the above into one queryable index.

## Failure modes

| Failure mode | Mitigation                                                                                                                                                                        |
|---|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Recorder run is slow on cold sqlite (first-time setup of zpwr's 17k completions) | Batched inserts, `PRAGMA journal_mode=WAL`, prepared statements; benchmark target: ≤2× normal startup time                                                                        |
| Stale records after a sourced file changes | Daemon's fsnotify (already wired) detects mtime change → triggers `zsh-recorder` re-run for the changed scope; old records of that file are flagged stale and excluded by default |
| Recorder + daemon down | recorder is opt-in; falls back to silently no-op; user sees "no records, run `zshrs-recorder`" message from `zwhere`                                                              |
| Schema growth across many runs | retain last K runs (K=10 default), GC older runs; daemon's existing log-rotation logic applies                                                                                    |
| `eval`-generated code overflowing record size | truncate `value` field at 4KB, store full content in a side blob keyed by sha256                                                                                                  |
| Sensitive content in records (export AWS_SECRET=...) | reuse `daemon/source_resolver.rs::is_sensitive` heuristic; `zwhere` masks values for sensitive entries unless `--show-sensitive` is passed                                        |
| Concurrent recorders | run_id is autoinc; multiple recorder runs are independent rows in `runs`; reads always see most-recent-by-default                                                                 |
| Hash collision on `current_defs` (same name set in multiple kinds) | primary key is `(kind, name)`, naturally handles                                                                                                                                  |
| Recorder catches its own internal calls | aspect's before-advice has a `IN_RECORDER` guard flag set during emit; nested calls during record-emit are skipped                                                                |

## Performance targets

| Metric | Target |
|---|---|
| Recorder run total time (zpwr + zinit + p10k init) | ≤ 2× regular zshrs startup |
| Per-event in-process capture cost | ≤ 5 µs (function-stack snapshot + struct push) |
| Batched IPC flush (512 events) | ≤ 200 µs round-trip |
| `zwhere` query (warm rkyv mmap) | ≤ 100 µs end-to-end |
| `zwhere` query (cold daemon) | ≤ 5 ms (sqlite query path) |
| Daemon RAM growth per recorder run | ≤ 2 MB resident (catalog + rkyv shard) |
| Disk per recorder run (typical zpwr config) | ≤ 4 MB (compressed sqlite + rkyv) |

## Implementation plan

### Phase 1 — single-aspect proof

1. Add `zshrs-recorder` binary.
2. Wire one aspect: `bin_alias` dispatcher. Before-advice captures (file, line, fn_chain) and pushes to in-process `Vec<RecordEvent>`.
3. At shell exit, dump the vec to stderr in plain text.
4. Run on `~/.zshrc`. Validate: every alias appearing in the user's interactive shell is in the dump, with correct file:line.

Estimated: 1-2 days.

### Phase 2 — full intercept surface

5. Add aspects for the remaining 14 builtin/syntax surfaces from the table above.
6. Validate each independently against a known config.
7. Add coverage test: `tests/recorder_corpus/*.zsh` — for each test file, list expected `(kind, name, line)` triples in a sibling `.expected` file; harness runs the recorder and diffs.

Estimated: 3-5 days.

### Phase 3 — daemon IPC + storage

8. Define `RecordEvent` IPC message in `daemon/ipc.rs`.
9. Recorder buffers + flushes on triggers above.
10. Daemon receiver: SQLite schema + insertion + rkyv shard build.
11. `current_defs` denormalized table maintenance.

Estimated: 3-4 days.

### Phase 4 — `zwhere` query builtin

12. Implement `zwhere` builtin in zshrs runtime, dispatching to daemon.
13. Output formatting + column stability for grep-ability.
14. `--lineage`, `--when`, `--file`, `--kind` flags.

Estimated: 2-3 days.

### Phase 5 — fsnotify-driven re-record

15. Daemon's fsnotify already exists; extend to trigger a recorder run on changes to any file in the most recent run's `definitions.file` set.
16. Stale-record marking + `zwhere` filtering.

Estimated: 2 days.

### Phase 6 — CI + fuzz

17. Property test: for any input file, `zshrs-recorder` followed by `zwhere all *` should return a record matching every alias/function visible in `whence`.
18. Fuzz: generated `.zshrc` mutations + `cargo fuzz` against the aspect layer to catch missed mutations.

Estimated: ongoing.

**Total Phase 1-5: ~2-3 weeks.** Reuses daemon / sqlite / rkyv / fsnotify already in place.

## Patent strategy alignment

This feature aligns with the existing patent strategy memory
(`aot_patent_strategy.md`):

- **Claim B** (zshrs-daemon architecture). The recorder is a new
  client class for the daemon — adds "metadata-recording client +
  cross-shell queryable index of every state-mutating shell event"
  to the daemon's responsibility set. This expands B's scope
  legitimately.
- **New dependent claim under claim B:** "Method for plugin-framework-agnostic
  attribution of shell-state modifications to source file and line
  number via runtime AOP intercept across the union of state-mutating
  builtin dispatchers." Specific novelty:
    - **runtime AOP** (vs static analysis) is novel for a Unix shell
    - **plugin-framework-agnostic** is novel — every existing tool
      (zinit-report, OMZ-list, antibody-list) is framework-coupled
    - **file:line + fn_chain + load-order** triple is the metadata
      surface no other tool exposes
    - **structured query layer** (`zwhere`) on the resulting index
      is novel — `set -x` is unstructured stderr; `whence` is
      schema-less; no shell has had a queryable definition index

The combination of (runtime-intercept × union-of-builtins ×
file:line × cross-shell-shared-via-daemon × queryable) hasn't been
shipped by any shell.

## Naming

- Builtin: `zwhere` (matches the `z*` builtin family per
  `cache_architecture_rkyv.md` memory)
- Mode flag: `zshrs-recorder`
- Wrapper: `bins/zshrs-recorder` (calls `zshrs-recorder`)
- Daemon op: `record_events` (matches existing `source_resolve`,
  `op_*` naming)
- SQLite table: `definitions` (canonical), `runs` (per-run header),
  `current_defs` (denormalized cache)
- rkyv shard: `~/.zshrs/recorder-{run_id}.rkyv`

## Open questions

1. **Should the recorder support a SHADOW mode** that emits records
   without storing? Useful for debugging the recorder itself or
   producing a one-off report without polluting the catalog.
2. **Should `zwhere` support fuzzy match** (`zwhere all g*t`)? Probably
   yes — sqlite GLOB is cheap.
3. **Should we capture function bodies in full** or hash + side
   blob? Bodies can be large (zpwr has 506 subcommands, total ~172k
   LOC). Default to truncate-at-4KB-with-blob-ref.
4. **Should we record `unalias` / `unset` / `disable` events** as
   well as definitions? Useful for tracking removal sites. Probably
   yes — adds 3 more aspects.
5. **How to handle module-level state mutations** (zsh modules like
   `zsh/datetime` setting up `$EPOCHSECONDS`)? Module load is one
   record; the parameters it provides are auto-discoverable via
   `parameter` module introspection — record once per module-load
   with a `provides` field listing exposed parameters.
6. **Cross-shell visibility:** if Shell A runs `zshrs-recorder` and Shell B
   queries `zwhere`, should Shell B see Shell A's records
   immediately? Yes — the daemon is the singleton. fsnotify on
   sqlite WAL gives sub-second visibility.

## Non-goals

- Real-time tracing in interactive shells (that's `set -x`'s job).
- Capturing variable READS (only writes/mutations).
- Replacing zinit-report — we coexist; `@zinit-report` shows ICE
  state, `zwhere` shows file:line for the resulting definitions.
- Cross-machine sync — recorder data is local to one daemon
  instance; cross-machine config tracking is out of scope.

## TL;DR

The **Plugin-Framework-Agnostic State-Modification Recorder
(PFA-SMR)**: a `zshrs-recorder` mode that, by AOP-intercepting every
state-mutating builtin dispatcher in zshrs's runtime, captures
`(kind, name, value, file, line, fn_chain, ts, prev_def_id)` for every
alias, function, export, fpath append, hash -d, zstyle, bindkey,
compdef, zmodload, setopt, trap, and sched in a user's shell config,
and forwards them to `zshrs-daemon`'s SQLite catalog + rkyv shard. A
new `zwhere` builtin queries the result.

**Foundational prerequisite:** zshrs is the first Unix shell with global
runtime AOP across the union of state-mutating builtin dispatchers.
The recorder is the first concrete application of that capability;
it is structurally impossible to ship in any other shell without
that shell first acquiring its own global-AOP layer (a multi-year
prerequisite project no other shell has attempted). This makes the
overall architecture a two-layer structural moat — the AOP layer
itself is a world-first, and the recorder built on top is a
world-first that nothing else can match without rebuilding the
foundation. See "Why this can only exist in zshrs" for the
exhaustive comparison table of every deployed Unix shell.

Four structural properties of the recorder:

1. **Plugin-framework-agnostic by construction — every framework,
   past and future.** No plugin-manager code is consulted. The
   dispatcher fires regardless of how the call got there — antigen
   (2010), zgen (2014), zplug (2014), prezto (2015), oh-my-zsh,
   antibody (2016), zinit (2017) including turbo + ICE, znap
   (2018), zsh-defer (2020), sheldon (Rust, 2021), zsh4humans (2022),
   raw inline `.zshrc` from any era, AND any framework that hasn't
   been written yet. They all funnel through zsh's finite,
   stable set of state-mutating dispatchers; the recorder intercepts
   at that bottleneck. Forward-compat is guaranteed by architecture,
   not by per-framework engineering effort.
2. **Zero recorder code in default `zshrs` binary.** Compile-time
   feature flag (`recorder`) gates every aspect, IPC type, and query
   path with `#[cfg(feature = "recorder")]`. Default build deletes
   them at the rustc-expansion stage; `nm target/release/zshrs |
   grep recorder` returns 0 — verified in CI as a release gate. The
   recorder ships as a separate binary, `zshrs-recorder`, from the
   same source tree under `--features recorder`.
3. **Strictly more information than every existing tool.** Beats
   zinit-report, oh-my-zsh introspection, `whence`, `which`,
   `functions -t`, `set -x`, `compaudit` — file:line + fn_chain +
   override-chain + load-order, structured and queryable.
   Specifically against zinit-report: the recorder is
   per-definition-granularity (one record per `alias`/`function`/
   `bindkey` call), not per-plugin-granularity (zinit collapses
   29 mutations into one diff summary). zinit-report fundamentally
   cannot recover per-definition file:line attribution because it
   diffs namespace snapshots rather than intercepting dispatchers;
   only intercept-at-definition-time gives provenance for individual
   mutations.

4. **Shell-mode-quirk elimination.** Interactive vs non-interactive,
   login vs non-login, sandbox/rzsh, command-mode, TERM variation,
   KSH/SH emulation — all dissolve under runtime AOP. The runtime
   resolves every conditional; the recorder observes the resulting
   state mutations; no static-analysis modeling of zsh's mode matrix
   is required. Each recorder run is tagged with the modes it ran
   in, so `zwhere` queries can target any combination.

5. **Shell environments as portable artifacts.** The end-state shard is a
   small (1-10MB), content-addressable, signable file. Shells can now
   be shipped, version-controlled, diffed, bisected, rolled back,
   signed, and distributed via registries — the Docker model applied
   to shell config for the first time. zpwr-as-artifact replaces
   zpwr-as-install-script. Reproducible shells across machines
   without dotfile drift; corporate blessed-shell publication;
   per-project shell snapshots; CI shells in 50ms instead of 5s.

15 aspects; bounded surface; existing daemon owns storage; 2-3 weeks
to ship Phase 1-5; novel against every shell that ships today.
