# Inventions

The canonical register of what originated in zshrs. Every entry here is
offered as **prior art for the shell-design commons** under the MIT grant —
see [`CREATORS.md`](../CREATORS.md) for the attribution ask.

## The filter

An entry earns a place only by passing all three tests:

1. **It exists.** Something in the tree does this, with a name you can type.
2. **Nobody else has it.** Not "we did it better," not "ours is compiled."
   No other shell does the thing at all, and the near misses are named.
3. **It's an idea, not a decision.** A file layout, a default, a flag, or a
   command name is a choice. An idea is something another project could
   inherit and be changed by.

Sixty-four candidates were considered — the earlier register plus everything
in the README and reference it left out. Twenty-nine survive. The cuts, with
reasons, are at the bottom; they are as much a part of this document as the
list, because a register nobody trims is a register nobody reads.

---

## Execution

### 1. Shell source compiled to bytecode, JITted to native, persisted across processes

Every command, script, function, and sourced file lowers to fusevm bytecode;
hot blocks go through Cranelift to x86-64/aarch64. Nushell reached bytecode
first (IR in 0.96.0, default in 0.98.0) but interprets it, rebuilds it per
parse, and drops it at exit. zsh's `.zwc` is wordcode for zsh's own
interpreter. Subtract both and what is left is: first shell to emit native
machine code, first to persist general bytecode across processes.

### 2. Native code persisted across launches

The `.fjit` disk cache keeps JIT output warm between shell invocations, so
hot chunks are not recompiled per process (`fusevm` features `jit`,
`jit-disk-cache`, `aot` — `Cargo.toml:123`; blobs are `{op_hash:016x}.fjit`,
with slot-kind vectors folded into the cache key so float-specialized code is
never reused for an integer slot). Strictly larger than entry 1. Managed
runtimes have done this for years — Android ART's `.oat`, .NET ReadyToRun,
JVM AppCDS — which is why its absence from every shell is the notable part.

### 3. Ahead-of-time compilation of the completion corpus, keyed on compiler identity

`--prewarm-autoloads` compiles every `$fpath` directory so a fresh install's
first `<TAB>` is a shard probe rather than a parse. Each entry is stamped with
the fpath directory, a SHA-256 of the exact definition text, and the identity
of the binary that compiled it. Hashing the compiler into the cache key is
standard in Nix, ccache, and Bazel, and absent from every shell cache. The
cache is deliberately bypassed for `ksh_autoload` bodies and `autoload`
without `-U`, where the compiled program is not a function of the file's
bytes alone.

### 4. JIT tier introspection as a diagnosis

`--tiers` runs a script, then asks fusevm's own predicates which tier took
each chunk, and for chunks that reached neither, lists the op kinds
responsible. Compilers have had optimization remarks for years. A shell
telling you *why* your loop stayed interpreted is new.

### 5. The anti-fork architecture

A persistent pool of 2–18 threads takes command substitution, process
substitution, globbing, completion, and autoloading; 23 coreutils commands run
in-process so a `cat` in a pipeline never leaves the shell. This is Baumann,
Appavoo, Krieger and Roscoe's "A fork() in the road" (HotOS 2019) applied to a
shell. The quoted 2000–5000x is fork overhead divided by a function call —
read it as a fixed cost disappearing, not as a benchmark.

### 6. Parallelism as grammar rather than library

`pmap`, `pgrep`, `peach`, `barrier`, `async`, and `await` are VM-dispatched
shell builtins (fusevm dispatch IDs 210–215), not library calls: the body
compiles once and runs on pooled VMs with zero forks. `make -j`, `xargs -P`,
and GNU parallel all sit outside the shell language. This is inside it.

---

## State

### 7. Recorder-owns-rebuild

A separate binary sources your real login chain while AOP interception sits on
every state-mutating dispatcher — `alias`, function definition, `export`,
fpath edit, `hash -d`, `zstyle`, `bindkey`, `compdef`, `zmodload`, `setopt`,
`trap`, `sched`, `source`, assignment — recording kind, name, value, file,
line, and call chain. Every other shell discovers your config by static
walking and hoping its parse matches the shell's. This is the DTrace insight
applied to configuration: watch what runs instead of reasoning about the
source. It is why `zwhere` can name the file and line that defined any alias
you have.

### 8. Splitting configuration into what can be cached and what must be replayed

`replay/` holds the non-deterministic `.zshrc` fragments. Everything the
recorder can prove is a pure function of your dotfiles folds into the
canonical shard and is skipped at startup; the rest is replayed. Purity
analysis on shell configuration is the load-bearing idea under the cold-start
number.

### 9. A singleton daemon owning every mutation, with stateless forkable clients

Canonical state, fsnotify, scheduling, jobs, locks, cache, history. No shared
writer, no auto-spawn. The pattern is emacsclient, Nailgun, and LSP; the
application to a shell's own state is new.

### 10. Session-persistent supervised jobs with bidirectional ptmx attach

`zjob submit --pty` opens a pseudo-terminal and runs the child on it under the
daemon. Attaching later puts your terminal in raw mode, pumps stdin in, drains
output back, forwards `SIGWINCH`, detaches on `Ctrl-]`. Collapses `nohup` +
`screen` + `pueue` + `disown`, and solves the case none of them solve cleanly:
a background job blocking on a prompt you cannot see.

### 11. Cross-shell pub/sub and token-issued named locks as builtins

`zsubscribe` / `zpublish` / `zlock`, routed through the daemon, replacing
hand-glued `flock` + `socat` + FIFOs. Release requires the original token, so
a dead shell cannot free someone else's lock.

---

## Observability

### 12. Value lineage at the bytecode level

`provenance` records an origin (command substitution, glob, heredoc, process
substitution) and every op that touched a value — expand, concat, assign,
exec, call — each stamped with file, line, enclosing function, and wall clock.
Functions carry the same chain: definition, every redefinition, every call at
the caller's line, the `unfunction` that ended it. One relaxed atomic load
when disarmed.

`typeset -p` answers what a value *is*. `set -x` answers what *ran*. Nothing
answers where the bytes came from. Weiser's slicing (1981) is static, Perl's
taint mode is one bit, PASS (2006) instruments the storage layer, W3C PROV
standardizes the model — all of it outside the shell, which is the layer that
actually glues the tools together. See [`PROVENANCE.md`](PROVENANCE.md).

### 13. Aspect-oriented advice on any command or function

`intercept` before/after/around with glob patterns, nanosecond timing, and
`intercept_proceed`. One primitive subsuming defer, profiling, memoization,
retry, and timeout. The vocabulary is forty-five years old — Flavors'
before/after methods (which were called daemons), CLOS `:around` in 1988,
AspectJ in 1997 — and no shell had offered it as anything but hand-rolled
function wrapping.

### 14. Lexer, wordcode, AST, and bytecode dumps on stdout

`--dump-tokens`, `--dump-wordcode`, `--dump-ast`, `--disasm`, plus
`--dump-reflection` for a JSON self-description of builtins, keywords, and
options. This is `javap`, Python's `dis`, and `perl -MO=Deparse` for a shell,
and no Bourne-family shell can describe itself to a program.

---

## Language

### 15. Sigil dispatch to a second language sharing the VM

`@{}` routes to embedded stryke, compiling to the same fusevm ops the shell
does — including inside AOP advice. Multi-language runtimes are the JVM and
CLR story, and Racket's `#lang` is the closest analogue for inline dispatch,
but no shell has hosted a second language in its own grammar.

### 16. A grammar extension with a switch that makes it vanish

`intercept … { }` is a real lexer extension, since zsh cannot parse a bare `}`
as an argument. The lexer captures the span as raw source, so the body keeps
its own redirections, pipelines, and heredocs, and nothing expands until the
advice fires. Under `--zsh` the extension is off and the form is rejected
exactly as `/bin/zsh` rejects it. This is how you add syntax to a language you
have also promised to be compatible with, and it generalizes past this
project.

---

## Extensibility

### 17. A stable, versioned, independently-published plugin ABI

`cargo add znative`, ship a `cdylib`, load it with `zmodload -R`,
version-gated with mismatches refused, only `#[repr(C)]` crossing the
boundary. Plugins register builtins and native completion generators wired
into compsys. `znative load owner/repo` is a package manager on top, building
on first start and loading from a content-addressed store thereafter.

bash's `enable -f` and zsh's `zmodload` load native code, but only against the
shell's private headers, with no stable ABI and no version gate, welded to one
build. That is why neither has third-party native plugins. Apache, nginx,
PostgreSQL, Redis, and Emacs 25 all published versioned boundaries and got
ecosystems. See [`PLUGINS.md`](PLUGINS.md).

### 18. Absorbing foreign binaries into the shell process

`git` served as a builtin with no fork, exec, or PATH lookup, and an
fzf-compatible finder honoring `FZF_DEFAULT_OPTS` in-process. BusyBox absorbed
the coreutils in 1995 and dispatched on `argv[0]`; this extends that logic past
coreutils into full applications. Named near misses: BusyBox has no git
applet, Nushell's `gstat` is status-only in a child process, `git-shell` execs
real git, and Elvish's in-process fuzzy modes are shell UI rather than a
pipeline filter.

---

## Compatibility

### 19. Two axes of emulation fidelity, both available

`--sh` reproduces `/bin/sh`; `--sh --zsh` reproduces zsh's *model* of
`/bin/sh`. Concretely, the bare POSIX modes use XSI `echo` where zsh's
`emulate sh` sets `BSD_ECHO`, and `--bash` accepts bash's own `set -o` names
including the six zsh has no option for. Eight Bourne-family dialects run
through one bytecode core, each verified against its actual reference shell
rather than against zsh's approximation. No precedent found for offering both
notions of fidelity as distinct modes.

### 20. A hybrid port: native spine, interpreted leaves, one tree

`src/compsys/ported/` mirrors zsh's `Completion/` layout exactly, but engine
functions are ported to Rust with citations while end-user completers are
copied verbatim alongside them — same filenames, dispatched through a
`_call_function` bridge: **991 upstream files mirrored verbatim, 249 engine
ports in Rust**, 1,240 files in total under that tree. The shape is a JIT with
an interpreter fallback, applied to a script framework.

### 21. Inheriting the configuration vocabulary of what you replace

The native ZLE engines are fish's Rust ports, but the palette and word-level
classification are fast-syntax-highlighting's, and `ZSH_HIGHLIGHT_STYLES`,
`ZSH_AUTOSUGGEST_*`, `HISTORY_SUBSTRING_SEARCH_*`, and `AUTOPAIR_*` all apply
unchanged. The p10k engine does the same with `.p10k.zsh`. The rule — when
replacing a widely-configured userspace layer with a native one, adopt its
config surface so nobody migrates — appears twice in the codebase.

### 22. Absorbing the prompt theme into the binary

powerlevel10k as 14 files and 15,479 lines of in-process Rust
(`src/extensions/p10k/`), 65 `*_segments` builder functions cited
line-by-line against the theme spec, `gitstatusd` replaced by a native
`.git` reader. Your `.p10k.zsh`
sources unchanged. The framing generalizes: instant prompt exists to hide the
cost of interpreting the theme, so when interpretation stops, the workaround
is deleted rather than ported.

### 23. Wall-clock budgets on per-keystroke rendering

Highlight and autosuggest passes are capped at 8 ms by default, so a huge
directory or a pathological `$PATH` cannot lag typing. Soft-real-time
scheduling applied to a line editor. Shell plugins have historically had no
budget at all, which is why a slow `git status` in a prompt hangs the
terminal.

---

## Verification

### 24. Architectural invariants enforced mechanically in CI

`tests/port_purity.rs` freezes the port directory: mirror a real
`Src/<x>.c`, cite the C function, no new files, no invented helper names.
`tests/tree_walker_absent.rs` asserts at source level that the old interpreter
is still gone, backed by 174 behavioral pins in
`tests/no_tree_walker_dispatch.rs`. The stated reason is drift — contributors
and code-generation tools both invent helpers, and both create files to hold
them — and both vectors are closed by a test rather than by review. This is
the mechanism that makes a large rewrite finishable.

### 25. Differential fuzzing a shell against a reference implementation

Grammar-driven, seed-replayable snippets per mode, run through both zshrs and
real zsh, divergences re-confirmed three times so load flakes cannot register.
22,200 cases across 74 modes, 27 divergences at 0.12%, 71 of 74 modes at zero,
each remaining mode traced to one named root cause (`bins/parity-fuzz.rs`,
[`PARITY.md`](PARITY.md)). McKeeman named differential testing in 1998,
Csmith made it standard for compilers in 2011, jsfunfuzz for JS engines.
Nobody had aimed it at a shell.

---

## Tooling

### 26. A source formatter in the shell binary

`--fmt`: block reindent, idiomatic spacing per the zsh and OMZ style guides,
heredoc-safe, idempotent, stdin-to-stdout or in-place. The same engine backs
the LSP's formatting capability, so editor and CLI cannot drift. `gofmt` made
this a language-community expectation in 2009; shells got `shfmt`, a
third-party Go program. First formatter inside a shell.

### 27. Live completion inside the editor

The LSP does not stop at builtins and in-file functions. It answers from the
real compsys completers, reading the recorded environment from the canonical
shard, so `git ch` completes `checkout` and `git checkout ` lists actual
branches — in your editor. Shell-out completers are killed at the request
deadline and never touch the editor's stdio. The architectural reason is the
whole point: because the server *is* the shell, it answers from the same
lexer, parser, option table, and completers the interactive session uses,
instead of a reimplemented parser in a separate Node project. See
[`IN_EDITOR_COMPSYS_COMPLETION.md`](IN_EDITOR_COMPSYS_COMPLETION.md).

### 28. LSP and DAP in the shell binary, and plugin state as IDE library roots

Scoped to what survives a prior-art check. Elvish shipped `elvish -lsp` in
0.18.0 on 2022-03-20 and Nushell shipped `nu --lsp` in 0.87.0 on 2023-11-14,
so "world's first shell with a language server" is false. Endo v0.1.0 shipped
`endo --lsp` and `endo --dap` together on 2026-04-08, five weeks before
zshrs's `src/extensions/lsp.rs` and `dap.rs` landed on 2026-05-16, so "first
with both" is false too. All three are outside the Bourne lineage.

What holds: **first shell in the Bourne lineage with either, and with both.**
bash, zsh, ksh, dash, and fish ship neither — `bash-language-server` and
`fish-lsp` are separate Node/TypeScript packages, and `vscode-bash-debug` is a
VS Code extension driving the external `bashdb` script. Alongside it,
`--dump-plugins` surfaces every sourced plugin from zinit / oh-my-zsh /
prezto / antidote / antigen / zplug under the IDE's External Libraries node —
cmd-clickable, find-usages-able, renameable across plugin boundaries. No shell
of any lineage exposes its plugin-manager state to an IDE.

### 29. A unit-test framework and runner in the binary

Fourteen assertion verbs plus a worker-pool runner: one persistent subprocess
per CPU, fork-on-receive per test, JSON over pipes, per-test fd-2 capture so
concurrent workers cannot tear each other's output. Bounded to
POSIX-compatible shells, with Pester named and excluded as non-POSIX and
Nushell's `std assert` named and excluded for lacking a runner. `bats-core`,
`shunit2`, Bach, ShellSpec, and zunit are all separate installs.

---

## Cut, and why

**Filing conventions (not ideas).** The single `~/.zshrs/` directory rule,
three log files one per binary, three auto-seeded TOML configs, the `z*`
naming convention, the loopback HTTP listener default. Each is a defensible
choice. A directory layout is not an invention, and putting four of them in a
twenty-nine-item register is what makes a reader discount the other
twenty-five.

**Commands and flags (not ideas).** `zd doctor`, `zd export`, `zd snapshot`,
`zd artifact`, `zsync up --all`, `zd` over two transports,
`barrier --fail-fast`, `--docs`. Useful surface. Nothing another shell would
inherit as a concept.

**Mechanisms of entries that survived.** rkyv shards (mechanism of 8 and 9),
the 8-file login chain (a correctness requirement of 7, not a separate idea),
the worker pool as its own entry (it is entry 5), VM pooling,
deparse-as-a-service, auto-derived OpenAPI.

**Not inventions at all.** "The `--no-fork` philosophy as a measurement" is a
review heuristic. Parity-first defaults are a stance. Publishing the
unflattering compatibility number instead of the flattering one is integrity,
and rare, and not a thing anyone ports.

**False.** The unqualified "world's first shell with a language server," and
"first shell with both LSP and DAP." See entry 28 for what survives.
Also cut: `par_for` / `par_map` / `par_filter` / `par_reduce` as named
parallel opcodes — the shipped primitives are `pmap`, `pgrep`, `peach`,
`barrier`, `async`, `await` (entry 6), and the four `par_*` names existed only
in the old register.

**Borderline, cut for consistency.** Flat history plus an FTS5 sibling index
refuses a real false choice and is good design reasoning. The 375 demo scripts
citing `Src/*.c` are executable documentation in the Knuth-to-doctests line.
Neither passes test 3.
