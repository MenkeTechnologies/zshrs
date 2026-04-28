# zshrs roadmap — endgame shell

**Status:** Phase F complete (tree-walker dispatch deleted, 96 hand-crafted tests pin "no tree walker" as load-bearing invariant). Phases G–O are the path from "no tree walker" to "v1.0 endgame shell, daily-driver replacement for zsh."

**Audience:** the maintainer (one person, planning in decades). Every phase has discrete acceptance criteria. No phase is complete until its tests pass.

---

## Operating principles

These are the design constraints. Violations are bugs.

1. **No tree walker, ever.** `execute_command` compiles to `fusevm::Chunk` and runs on `fusevm::VM`. The deleted dispatch tree (`execute_simple/pipeline/list/compound/command_bg`) does not return. `tests/tree_walker_absent.rs` enforces.

2. **Power-user defaults are non-negotiable.** No startup banner, no init-progress to terminal, no deprecation nags, no "did you mean," no safety prompts on destructive ops, no tip-of-the-day, no `--help`-hints in errors. Errors stay on stderr in `zshrs: <cmd>: <reason>` format. Informational chatter goes to `~/.cache/zshrs/zshrs.log` via `tracing::*`. One-way ratchet — never reintroduce removed banners. **This is not fish.**

3. **Every PR ships behavioral tests.** Pin exact stdout + exit. The 96 tests from Phase F are the floor; this plan adds 400+. No invariant, no merge.

4. **Compat with the maintainer's existing world is sacred.** zpwr (172k LOC), zsh-more-completions (16,806 files), the .zshrc, all zinit plugins — these continue working as zshrs evolves. Compat-floor regressions are catastrophic; ship none.

5. **Plan in decades.** This is the endgame shell for a 30+ year horizon. Every architectural decision must survive that. Bytecode formats versioned. SQLite schemas migration-safe. Dependencies vendorable. MSRV pinned. No fashionable churn.

6. **Readable over clever.** Future-maintainer-at-65 must navigate this codebase. Concise, documented, no premature optimization that obscures intent.

7. **zshrs is the god of all processes.** Default new features to "lives inside zshrs as a builtin or host op." External processes only when there's no choice. Forks cost 2-5ms; millions of commands per day; every fork avoided is real wall time.

8. **Synthesis posture, not migration posture.** Anything good fish/nushell/elvish/oil innovate, evaluate for absorption. Endgame means convergence, not just zsh-in-Rust.

---

## Phase G — Compat floor (daily-driver replacement)

Critical path. Until G is done, zshrs cannot replace zsh on the maintainer's machine. Order is dependency-respecting.

### G0 — Friendly-output audit
- Sweep every `println!`/`eprintln!`/`eprint!`/`print!` in the codebase.
- Route each to `tracing::{info,warn,debug}` UNLESS it's in: `--help`/`--version`/`--doctor`/`dbview`/builtin user-invoked output, error reporting on stderr (`zshrs: <cmd>: <reason>` format only), or user-script output.
- Add `#![deny(clippy::print_stdout, clippy::print_stderr)]` lint to `bins/zshrs.rs` so future regressions break the build.
- **Acceptance:** `./target/debug/zshrs -i -f </dev/null` produces zero bytes on stdout/stderr before the prompt.
- **Test:** regression test runs `zshrs -i -f` with stdin closed, asserts output is empty until prompt char.
- **Effort:** 1 day.

### G1 — Real shell arrays
- `compile_simple` detects `ShellWord::ArrayLiteral(elems)` in assignment context, emits N pushes + `Op::MakeArray(N)` + `BUILTIN_SET_ARRAY` (id 287).
- New `BUILTIN_SET_ARRAY` writes `Vec<String>` into `executor.arrays`, not `executor.variables`.
- New `BUILTIN_SET_ASSOC` (id 288) for `declare -A` assoc arrays.
- `compile_word` for `ShellWord::ArrayVar(name, idx)` lowers to `BUILTIN_ARRAY_INDEX` (id 289).
- `ZshrsHost::expand_param` handles `ArrayLength`/`ArrayIndex`/`ArrayAll`/`KEYS` (`${arr[@]}`, `${#arr[@]}`, `${!arr[@]}`).
- **Argv splice (the hard part):** bump fusevm to 0.10.1 — modify `Op::Exec`/`Op::ExecBg`/`Op::CallFunction` to flatten `Value::Array` arguments into argv (one-line `flat_map` semantics change).
- 30+ array tests in `no_tree_walker_dispatch.rs`: indexing, slicing, splice expansion, assoc keys/values, `${arr[@]}` in for-loop, `${arr[@]}` as cmd args, append `arr+=(...)`, `${#arr}` length.
- **Acceptance:** every zsh array idiom in zpwr's source compiles + runs identically.
- **Effort:** 3 days.

### G2 — Autoload + fpath scan via sharded rkyv image cache
- Per `AOT_DESIGN.md` §0x13: daily-driver bytecode lives in `~/.cache/zshrs/images/{shard}.rkyv` (one image per source root) with top-level `~/.cache/zshrs/index.rkyv` for two-level lookup. Wire `fn autoload_function` (exec.rs:2907) to: hash fq_name → `index.rkyv` lookup → `(shard_id, byte_offset)` → get-or-mmap the shard image (LRU shard-handle cache) → typed pointer → JIT/interp the chunk in place. ~150-200ns end-to-end. No SQLite hit on the hot path.
- Per-shard rebuild: `fsnotify` watches each known source root; on source change, recompile only that root's image, atomic-rename, update `index.rkyv` (small file rewrite), enqueue per-shard catalog hydrate. `git pull` in zpwr rebuilds only `zpwr.rkyv` (~3-5s); `zinit update foo` rebuilds only `plugin-foo.rkyv` (~100-500ms).
- Worker pool hydrates `~/.cache/zshrs/catalog.db` per-shard (`DELETE FROM entries WHERE plugin_id = '{shard}'` then INSERT). `entry_stats` survives shard rebuilds via `ON CONFLICT DO UPDATE` keyed on `fq_name` so personal usage analytics aren't reset every plugin update. Hydration logs to `~/.cache/zshrs/zshrs.log` via `tracing::info!`. Never on hot path; dbview consumer only.
- Legacy `plugin_cache.db` / `compsys.db` SQLite caches and `.zwc` reading paths are dead-on-arrival for new investment — see "What dies" in `AOT_DESIGN.md` §0x13.
- **Test:** install a plugin via `zshrs --rebuild-cache --shard plugin-foo`, call its function, verify it resolves through the rkyv path (not tree-walker) by asserting the dispatch hits the index → shard mmap chain. dbview returns the entry within ≤ shard rebuild duration. Edit one zpwr function, verify only `zpwr.rkyv` (not the full corpus) rebuilds.
- **Acceptance:** full-corpus rebuild <30s clean; per-shard rebuild ~3-5s for large shards (zpwr, completions-corpus) and ~100-500ms for single plugin shards; cold shell launch <5ms; 16 parallel shells share <30 MB RSS attributable to images; Tab completion lookup ~150-200ns.
- **Effort:** 6 days (sharded image format + index.rkyv + perfect-hash + LRU shard-handle cache + per-shard worker hydrate + fsnotify wiring + dbview integration).

### G3 — ZLE hooks + user widgets + completion firing
- `zle/main.rs:496` (hook system stub): implement `precmd`/`preexec`/`chpwd` hook dispatch — call `executor.hook_functions[name]` via `execute_command` (now bytecode).
- `zle/main.rs:553` (user widget execution): when `bindkey '^X^E' my-widget` is bound, hitting that key invokes `my-widget` via `execute_command(ShellCommand::Simple([Word::Literal("my-widget")]))`.
- `zle/main.rs:646` (prompt expansion): use `expand_prompt` from `prompt.rs` which exists; the gap is the wiring.
- `zle/hist.rs:196` (incremental search UI): real `Ctrl+R` impl via reedline.
- Completion: when user types Tab in line editor, fire bound widget which calls the user's completion function (via `compdef name handler`); handler runs through bytecode VM, populates `executor.comp_matches`, line editor reads them.
- **Tests:** keypress simulation tests that bindkey a widget, simulate the keystroke, assert widget ran and side effects landed.
- **Acceptance:** tab-completing `git ` in zshrs produces the same matches as zsh (both backed by zsh-completions corpus).
- **Effort:** 4 days.

### G4 — zstyle, zmv, zparseopts, parameter flags
- `zstyle ':completion:*' completer ...`: builtin exists at `BUILTIN_ZSTYLE` (id 140) but the compsys-side reader needs to actually consult it.
- `zmv 'foo*.txt' 'bar*.txt'`: autoload from fpath, rebuild as bytecode.
- `zparseopts -A opts -- a b: c::`: builtin exists (id 147), tests needed.
- Parameter flags: `${(L)var}`, `${(U)var}`, `${(j: :)arr}`, `${(o)arr}`, `${(f)$(cmd)}`, `${(s. .)var}`, `${(@)arr}`, `${(k)hash}`, `${(v)hash}`, `${(P)var}`, `${(qqq)var}`.
- Each flag: extend `ZshrsHost::expand_param` with new `param_mod` constants OR route through existing `BUILTIN_EXPAND_WORD_RUNTIME` (acceptable for less-hot flags).
- **Tests:** 50+ parameter-flag combinations.
- **Acceptance:** zpwr's parameter-flag-heavy lines work identically.
- **Effort:** 3 days.

### G5 — Glob qualifiers
- `*(.x)` (executable), `*(N)` (nullglob), `*(/)` (dirs), `*(@)` (symlinks), `*(.)` (regular files), `*(L+1024)` (size > 1KB), `*(mh-1)` (modified < 1 hour), `*(om[1,5])` (oldest 5).
- Modify `executor.expand_glob` to parse `(qualifier)` suffixes and filter results.
- The compile-time path: `compile_word` for Literal containing `(` after `*` routes to runtime fallback for now.
- **Tests:** 30+ qualifier tests against the local filesystem with controlled fixtures.
- **Acceptance:** every qualifier zsh manual lists works.
- **Effort:** 2 days.

### G6 — Job control parity
- `jobs`, `fg`, `bg`, `wait`, `disown`, `kill %1` — builtins exist (ids 50-56).
- Background `&`: `compile_list` ListOp::Amp currently runs sync (TODO at line 404); emit a fork-or-thread that detaches, register PID/thread-handle in `executor.jobs`, return immediately.
- Job state transitions: SIGTSTP → Stopped, SIGCONT → Running, SIGCHLD → Done; `tcsetpgrp` for foreground swap.
- Signal forwarding: SIGINT to foreground job only, SIGWINCH to all jobs.
- **Tests:** spawn sleeping job, suspend with Ctrl+Z, list jobs, fg, verify state machine.
- **Acceptance:** `vim &; jobs; fg %1` works identically to zsh.
- **Effort:** 3 days.

### G7 — Signal traps (DEBUG, ERR, EXIT, ZERR)
- `trap 'echo bye' EXIT`: builtin id 100 exists; the dispatch is `Op::TrapSet(idx)` which the host's `trap_set` should record + fire on appropriate signal.
- DEBUG trap: fires before each command; emit `Op::TrapCheck` between commands in compile_simple.
- ERR trap: fires when last_status != 0; check after every SetStatus.
- EXIT trap: fires at script end; run before VM halts.
- ZERR trap: zsh-specific, fires on any error.
- **Tests:** each trap type with assertions that the trap fired exactly N times.
- **Acceptance:** zsh's `traps` testfile (W08traps) passes.
- **Effort:** 2 days.

### G8 — Kill `BUILTIN_EXPAND_WORD_RUNTIME` (the last residual dup)
- After G1 (arrays) and G4 (parameter flags) land, the only remaining users of `BUILTIN_EXPAND_WORD_RUNTIME` (the JSON-AST runtime fallback that delegates to tree-walker-era `expand_word_glob`) should be: mixed `$VAR + glob` literals, and any edge cases discovered during dogfood.
- Lower the mixed-literal case to ops: chain `Op::ExpandParam` + `Op::Glob` + `BUILTIN_ARRAY_JOIN` for literals containing both `$` triggers and glob chars.
- Audit all remaining call sites of `compile_word_runtime`. Any that exist after the lowering should either get a native op or be a documented "this case can't happen" assertion.
- Delete `BUILTIN_EXPAND_WORD_RUNTIME` (id 281), delete `compile_word_runtime` helper, delete the JSON-roundtrip serialization path entirely.
- `expand_word_glob` and `apply_var_modifier` either get inlined into the relevant host methods (where they're called by `Op::ExpandParam`) or deleted — they're tree-walker-era code that should not survive once nothing calls them.
- **Acceptance:** grep for `BUILTIN_EXPAND_WORD_RUNTIME` returns zero matches in src/. `cargo build` succeeds with zero warnings about unused functions in the expansion module.
- **Why this matters:** until G8 is done, zshrs has dual *implementations* of word expansion (native ops vs runtime fallback delegating to tree-walker code) even though dispatch is unified. That residual dup is the last debt from the rip-out. After G8, zshrs has literally one code path for every shell construct — no fallback, no alternative, no "if X then bytecode else tree walker" anywhere.
- **Effort:** 2 days after G1+G4 land.

**Phase G exit criteria:** maintainer's `.zshrc` + zpwr loads cleanly in zshrs with zero errors and zero behavioral differences from zsh. **Plus:** `BUILTIN_EXPAND_WORD_RUNTIME` is deleted from the codebase. zshrs has literally one code path for every shell construct.

---

## Phase H — Verification (dogfood)

### H1 — `.zshrc` + zpwr load test
- New test file `tests/dogfood_zshrc.rs`: checks out (or symlinks) the real `.zshrc` and zpwr; runs zshrs with `-c 'source ~/.zshrc; echo READY'`; asserts `READY` is in stdout, exit 0.
- Captures stderr to a log file; any non-empty stderr fails the test.
- Run on every CI build.
- **Effort:** 1 day.

### H2 — zpwr 506+ subcommands functional test
- Generate a test case for each zpwr subcommand: `subcmd --help` produces non-empty output and exits 0.
- Bonus tests for the 50 most-used subcommands (from zsh history frequency): verify their actual functionality.
- **Effort:** 3 days.

### H3 — 16,806 completion smoke test
- Iterate every `_*` completion file; ensure it loads (autoload + bytecode compile) without errors.
- For 100 most-common commands, verify `compdef` registered the handler.
- **Effort:** 1 day.

### H4 — `dogfood -live` mode
- 30-day rotation: alternate days using `chsh -s zshrs` and `chsh -s zsh`.
- Log every error hit in zshrs to a known file.
- Each error becomes a regression test.
- **Effort:** 30 days calendar (parallel with other phases).

**Phase H exit criteria:** 14 days of zshrs as login shell with zero panics, zero hangs, zero command-output differences vs zsh.

---

## Phase I — Storage durability (endgame schema)

### I1 — Bytecode chunk format versioning
- Prepend version byte (currently `0x10` for v0.10.0 fusevm format) to every serialized chunk in SQLite.
- On read: if version mismatches current, silently invalidate cache row, recompile from source on next access.
- **Test:** write a v0.9.x-format chunk to compsys.db manually, verify zshrs gracefully rebuilds.
- **Effort:** 1 day.

### I2 — SQLite schema migrations
- Each cache db (`compsys.db`, `plugins.db`, `bytecode.db`, `history.db`) gets a `schema_version` table.
- On open: check version; if older, run migration scripts; if newer, fail loudly (downgrade is unsafe).
- **Test:** open a v0 db with v1 zshrs, assert clean migration.
- **Effort:** 1 day.

### I3 — MSRV + Cargo.lock policy
- Pin MSRV in Cargo.toml `rust-version = "1.75"` (or whatever current minimum is).
- Commit `Cargo.lock`.
- CI matrix: build against MSRV + stable + beta to catch toolchain regressions early.
- **Effort:** 0.5 day.

### I4 — Dependency audit
- `cargo audit` in CI.
- Review every dep in Cargo.toml: who maintains it, last release, alternatives.
- Vendorable test: `cargo vendor && cargo build --frozen --offline` must succeed.
- **Effort:** 1 day.

---

## Phase J — Test invariant ratchet (parallel with G)

Each G item ships with its own tests. Phase J is the additional sweep beyond per-phase tests.

### J1 — setopt flag matrix
- Every zsh option (`AUTO_CD`, `EXTENDED_GLOB`, `NULL_GLOB`, `HIST_IGNORE_DUPS`, etc.) has a test that toggles it on, performs an action, asserts behavior. Same with off.

### J2 — Glob qualifier exhaustive
- Every qualifier in `info zsh "Glob Qualifiers"` has a test.

### J3 — Parameter flag exhaustive
- Every flag in `info zsh "Parameter Expansion Flags"` has a test.

### J4 — Redirect form exhaustive
- All 9 redirect ops × all 4 here-doc/string variants × file/fd target = ~50 tests.

### J5 — POSIX conformance subset
- Run a subset of Open POSIX Test Suite shell tests; track pass rate as a metric over time.

**Target:** 500+ behavioral tests by phase J completion.

---

## Phase M — Performance proof

### M1 — Hyperfine harness in CI
- New crate `bench/` with hyperfine-driven scripts comparing zshrs vs zsh vs bash on:
  - Cold startup (`time $shell -c true`)
  - Warm startup (`time $shell -c 'source ~/.zshrc; true'`)
  - Pipeline (`echo $(seq 100) | tr ' ' '\n' | sort | uniq | wc -l`)
  - 100 builtin calls (`for i in {1..100}; do echo $i; done`)
  - Glob expansion (`echo **/*.rs` over the repo)
  - compinit (`time autoload -Uz compinit; compinit`)
- **Effort:** 1 day.

### M2 — Publish numbers
- Run on actual machine (M-series Mac, real .zshrc).
- Update README perf table with hyperfine output.
- Retire any unverified claim in README/RFC; replace with measured numbers.
- **Effort:** 1 day.

### M3 — Regression alarm
- CI runs the bench on every merge; any regression > 10% fails CI.
- **Effort:** 0.5 day.

**Phase M exit criteria:** README's performance claims are reproducible from a single command. No vapor numbers.

---

## Phase K — Cross-platform CI

### K1 — GitHub Actions matrix
- macOS aarch64 (daily driver)
- macOS x86_64 (Intel Macs still in use)
- Linux x86_64 (servers)
- Linux aarch64 (Graviton, RPi)
- FreeBSD x86_64 (optional, low priority)
- **Effort:** 1 day.

### K2 — Per-platform behavioral tests
- Some tests are macOS-specific (`/usr/bin/true` exists, `/bin/true` doesn't).
- Some are Linux-specific (procfs, different default PATH).
- Tests gated with `#[cfg(target_os = ...)]` where needed.
- **Effort:** 1 day.

---

## Phase L — Synthesis features (after compat floor solid)

These features absorb the best of other shells. None ship before Phase G is done.

### L1 — Fish-style syntax highlighting in ZLE
- Already partially in `fish/` subtree; finish wiring into reedline highlighter.

### L2 — Fish-style autosuggestions
- History-based; SQLite history backend already exists; just needs the inline-render hook.

### L3 — Atuin-style history search
- `history.db` is SQLite + FTS5; expose `Ctrl+R` that does FTS query, not substring.

### L4 — nushell-style structured data via stryke
- `ls | from nu | where size > 1MB` — pipe is JSON between zshrs and stryke.

### L5 — Abbreviations (zsh-abbr / fish abbr)
- Type `gco`, space → expands to `git checkout`.
- ZLE-level feature; partially in `fish/abbrs.rs`.

---

## Phase N — Documentation honesty pass

### N1 — README rewrite
- Every claim has a test or benchmark backing it.
- Drop "100% compiled" until it's literally true (Phase G7 done).
- Drop "100x" perf numbers until M2 publishes real numbers.
- Add "what works today / what's coming / what zsh does that zshrs doesn't yet."

### N2 — RFC update
- Same honesty pass.
- Update phase status from aspirational to measured.

### N3 — Architecture doc
- New `docs/ARCHITECTURE.md` covering: VM dispatch, ShellHost callbacks, SQLite caches, worker pool, fork-per-stage pipelines.
- Diagram-heavy, code-references-by-line-number for navigability.

### N4 — Plugin author guide
- For people writing zshrs-aware plugins: AOP intercept usage, async/await/pmap, dbview, profile builtin.

### N5 — Migration guide for zsh users
- Side-by-side: "this zsh idiom works the same / works differently / doesn't work yet."

---

## Phase O — Release v1.0

### O1 — Packaging
- Homebrew formula
- AUR PKGBUILD
- Nixpkg
- Debian deb (lower priority)

### O2 — Stable API guarantee
- v1.0 means: bytecode format frozen, builtin dispatch table frozen, SQLite schemas frozen with migration support.
- Future breaking changes go in v2.0 with migration path.
- Semver discipline going forward.

### O3 — Bench data + announcement
- Blog post / GitHub release with M2 numbers.
- Cross-link from zpwr README.

### O4 — Adoption metrics
- Track crates.io downloads, brew formula installs, GitHub stars.
- Not vanity — early signal of who else is finding it useful.

---

## Realistic timeline

| Phase | Items | Wall time | Calendar (part-time) |
|-------|-------|-----------|----------------------|
| G | G0–G7 | 17 days work | 4–8 weeks |
| H | dogfood | 30 days calendar | parallel with G/I |
| I | storage durability | 4 days | parallel with G |
| J | test ratchet | continuous | every PR |
| M | perf bench | 2 days + ongoing | week 6+ |
| K | cross-platform | 2 days | week 8+ |
| L | synthesis | 2-3 weeks | post-Phase G |
| N | docs | 5 days | week 10+ |
| O | release v1.0 | 1 week | month 3-4 |

**Total:** ~3-4 months calendar to v1.0 endgame. Honest estimate assuming part-time work on top of day job. Full-time would be ~6 weeks.

---

## Today actions

1. **G0 audit + lint** — sweep stops the friendly-output regression class permanently. 2 hours.
2. **Open Phase G1 (arrays)** — the highest-leverage next move. Branch off, draft fusevm 0.10.1 with the argv-splice change, write the array test corpus first (TDD), then implement until tests pass.
3. **Decide priority of Phase H1 dogfood** — can be started in parallel even with current array breakage by using a stripped-down `.zshrc` that avoids arrays. Surfaces more bugs sooner.

---

## What remains aspirational until measured

These claims appear in README/RFC but are not yet backed by reproducible benchmarks. They get retired or substantiated in Phase M.

- "100x warm start speedup"
- "2000-5000x cat fork avoidance"
- "7x CI/CD pipeline speedup"
- "100-400x script startup vs bash/zsh"
- "180+ builtins" (count needs verification)
- "23 coreutils builtins" (verified — listed in README)
- "129 fusevm opcodes" (count needs verification post-0.10.0)

Phase M produces measured replacements. Until then, the README's "performance" section reads as architecture description, not benchmark report.
