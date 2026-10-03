# Post-Parity Hot-Path Optimization Backlog

Status: optimization backlog for the phase that follows zsh parity. The items
below are **deferred optimizations, not defects.** The port is faithful and
correct by design; it is unoptimized on purpose, for now.

## Phase and purpose

zshrs is in the **faithful-port, correctness-first phase.** The port copies
zsh's C semantics — including zsh's global-mutable-state architecture —
faithfully, so parity can be reached and pinned by the test suite. Performance
is deliberately deferred, and that sequencing is correct:

- You cannot safely optimize an implementation whose semantics are still
  moving. Every parity fix that changes *what* a function returns would
  invalidate an optimization of *how* it returns it.
- An optimization is only safe once a green parity suite can certify it did not
  change behavior. Until the floor is complete, that certificate doesn't exist
  for much of the surface.
- Optimizing before parity risks the compat floor for speed before the floor
  is even finished — backwards.

So: **faithful first, guarded optimization second.** This document is the
backlog for that second phase. Each entry is `file:line`-cited and prioritized
so it can be picked up one at a time, each change guarded by the parity suite.
An entry is "make this hot function lean **without changing what it returns**,"
never "change the contract."

## The engine is not the problem — stryke proves it

stryke runs on the **same fusevm VM and the same Cranelift JIT** as zshrs, and
it crushes zsh. Same engine, opposite result. That single fact exonerates the
infrastructure: the VM, the JIT, and the thread pool are not the deferred cost.
The deferred cost is entirely in the **port layer's per-operation mechanism** —
which is exactly where a correctness-first port accrues it.

## The measured gap (current phase)

Interactive shell code is a firehose of tiny operations — a pattern match, a
parameter read, a substring strip, an option check — millions of them, each
dispatching into a `src/ported/` function that was written for correctness, not
per-call leanness. Measured, `zshrs --zsh` vs system `zsh 5.9.1`, same input:

| Workload | zsh | zshrs | Ratio |
|---|---|---|---|
| `[[ x == (#b)prompt_…(*) ]]` ×20000 | 0.02 s | 1.53 s | 76× |
| `${(M)a:#t*}` ×20000 | 0.04 s | 1.68 s | 42× |
| `case $x in foo*)` ×20000 | 0.01 s | 0.75 s | 75× |
| `${+functions[f5]}` ×3000, 3000 fns | 0.01 s | 15.89 s | ~1590× |
| p10k first precmd (real config) | ~1 s | >90 s | >90× |

None of this is the engine (stryke would be slow too). All of it is deferred
port-layer cost.

## The root cause is one architecture, and its fix is deferred

zsh's C is **global mutable state by design** — `paramtab`, `shfunctab`,
`patout`, `patparse`, every `Src/*.c` file-scope static. That is *fast* in C
because C is single-threaded: the globals are never contended, reads are a
load, and everything is arena/buffer-based. The faithful port kept that
structure (same globals, same statics), which is the correct thing to do for
parity.

Then the port is run under a parallel test harness and a worker pool, and
**global mutable state under threads is a data race.** So a lock went on every
global to make it thread-safe. The result — for now — is the antithesis of
parallel: **serial execution plus atomic overhead.** Locking global mutable
state does not parallelize it; it serializes every access and adds the lock
tax. And the locks exist primarily so the **parallel test harness** can hammer
shared globals without UB — a test-infrastructure need leaking into the runtime
cost of every operation, with no runtime speedup to show for it.

This is not a mistake to regret; it is the expected state of a faithful port of
a global-state interpreter. The **fix belongs to the optimization phase**, and
it is not "make the globals atomic":

- Concurrent maps (dashmap/flurry/evmap) still pay per-access atomics and need
  epoch/hazard reclamation — a GC-adjacent scheme the project forbids. Wrong
  direction.
- The right move is to make the hot-path state **not global-shared**: the C
  globals become fields of an owned per-execution context (or `thread_local!`
  as the minimal-diff intermediate), owned by the one thread that runs the
  sequential statement stream, **unlocked**. Tests then parallelize by
  **isolation** (each test its own context, sharing nothing) — which is the
  correct way to parallelize tests and deletes the reason the locks exist. The
  worker pool operates on **independent data** — a compiled chunk, a cache
  shard, an `&` branch on an immutable `Arc` COW snapshot — never on the live
  tables.
- This is coupled to running fine-grained ops **inline** on the owner thread
  (backlog #6): thread-confined state only works if the statement stream stays
  on its owner thread. The two are one move — the exec thread runs the
  sequential stream and owns its state locklessly; the pool does coarse
  independent work.

The faithful-port rule survives this intact: you port the state *structure*
faithfully (same fields, same semantics) and change only its *storage* — from a
locked global static to a context the owner holds. That is exactly what a
reentrant/threadable C rewrite would also have done.

## The template — how every backlog item is done (pattern cache)

The compiled-pattern cache (`src/extensions/pat_cache.rs`) is the first item
taken from this backlog, and it is the template for the rest — an optimization
done without touching the contract:

- It sits **behind the faithful interface** (`patcompile`); callers are
  unchanged.
- It lives in **`src/extensions`**, not in the ported semantics.
- **Parity is provably unchanged** — the pattern test suite has the same 7
  pre-existing failures before and after, and the cache key is option-sensitive
  (`extendedglob`/`kshglob`/`shglob`/`caseglob`/`casepaths`/`multibyte`), so
  `[[ … ]]` under any option toggle still matches zsh byte-for-byte.
- **Zero behavior change, pure speed** (halved p10k precmd CPU).

Every backlog item takes this shape: change the *implementation*, never the
*contract*, and let the parity suite certify it.

## The backlog

Ordered by leverage. Each is a deferred optimization; C's mechanism is the
target the port converges to once its area is at parity.

### Steady-state dominators (hit by every command, not just startup)

**#6 — Fine-grained ops dispatched to the worker pool.** Profiling
`[[ x == a*f ]]` put `thread`/`worker`/`spawn` frames at the top. Cond eval and
small builtins are handed to the 18-thread pool; the hand-off costs more than
the work, and it defeats thread-confined state. *Target:* run tiny ops inline
on the owner thread; reserve the pool for units big enough to amortize a
hand-off. This is the largest single item and is coupled to the state-ownership
move.

**#10 / #11 — A global lock on every symbol lookup.** Every core symbol table
is a global `RwLock`/`Mutex` locked per access, where C uses an unlocked
single-threaded hash:

- `paramtab` — `RwLock` (params.rs:13002), **108** lock sites → every `$var`.
  `getsparam` (params.rs:4654) takes two `RwLock::read()` (its own + `is_nameref`
  at 16736) plus a value clone per read. C: one hash probe returning a pointer.
- `shfunctab` — `RwLock` (hashtable.rs:3229), ~36 sites → every function call.
- `aliastab` — `RwLock` (hashtable.rs:3172), ~18 sites (incl. `lex.rs`) → every
  command word.
- `cmdnamtab` — `RwLock`, ~15 sites → every external command.

Even uncontended, a `RwLock::read` is an atomic RMW + fences (~30-80 ns) vs C's
~2 ns load. *Target:* per-context owned tables, unlocked; reads return `&str`
borrows, not clones.

**#12 — Function-call frame under a lock.** Each call locks `shfunctab` for
lookup and pushes/pops `FUNCSTACK` behind a `Mutex` (exec.rs:5867/5892/6027). C:
a bare linked-list push/pop. *Target:* owner-thread funcstack, no lock.

**#13 — An allocation per expansion.** `singsub`/`multsub` (subst.rs:1452/1530)
allocate a fresh `LinkList` + `push_back(s.to_string())` per `${…}`/`$(…)`. C
reuses a per-command list and works on metafied bytes in place. *Target:*
reused list / in-place bytes.

### Startup dominators (why p10k precmd specifically is slow)

**#1 — Whole-table materialization of magic assocs.** `assoc_get`
(subst.rs:17046) builds the entire magic-assoc `IndexMap` (for `functions`,
enumerate ~50k names + reconstruct every body) to serve a single-key or
existence access, with an O(N) bucket-reorder. **34** call sites; only the
`${+…[key]}` existence path (7807) is done. C: `getnode(ht,key)`, one probe.

**#2 — Function-body reconstruct per read.** `${functions[name]}` re-parses and
re-deparses the body every read (`parse_string`+`getpermtext`,
parameter.rs:1100-1103). C keeps the compiled `Eprog` and deparses from the
arena.

**#3 — Pattern compile under global locks with `Mutex` static buffers.** All
**65** `patcompile` callers recompile; the compile takes `PATCOMPILE_LOCK`
(pattern.rs:3834) and uses global `Mutex` statics `patout` (214) / `patparse`
(3839) as shared buffers, plus a fresh `String` decode per compile.
*Mitigated* by the extension cache for repeats; the first-seen compile is still
fat. *Target:* per-call arena, thread-confined compile state.

### The rest

**#4 — Double-compile per `[[ … ]]`** — validate (fusevm_bridge.rs:7252,
discarded) then match (vm_helper.rs:4250). C compiles once.

**#5 — Whole-array clone for single-index reads** — `arrays_get(&var_name)`,
**63** sites, clones the array then often takes one element. C indexes in place.

**#7 — Global-option thrash per pattern match** — `matchpat`
(glob.rs:2054-2072) does 4 `opt_state_set` (+ opts-cache invalidations) per
`[[ ]]` to pass case/extended sensitivity through globals. C passes them as
arguments.

**#8 — O(N log N) sort per magic-assoc enumeration** — `${(k)functions}` etc.
`names.sort()` at **9** sites (subst.rs:8326…8470). ~50k entries per
enumeration; also diverges from zsh's hash order.

**#9 — Re-tokenize the pattern per match** — `glob::tokenize` again before
compile, **32** sites in subst.rs, **25** in pattern.rs. C tokenizes once at
parse time.

### Inventory at a glance

| # | Site | C cost | Port cost (deferred) |
|---|---|---|---|
| 1 | `assoc_get` magic-assoc (34) | 1 hash probe | materialize whole table + reconstruct bodies |
| 2 | `${functions[name]}` read | arena deparse | re-parse + re-deparse body |
| 3 | `patcompile` (65 callers) | arena ~1µs, no lock | global lock + `Mutex` static buffers + realloc |
| 4 | `[[ … ]]` compile | once | twice (validate + match) |
| 5 | `arrays_get` (63) | index in place | clone whole array |
| 6 | fine-grained op dispatch | inline call | worker-thread hand-off |
| 7 | `matchpat` options | pass as args | 4 `opt_state_set` + invalidations / match |
| 8 | `${(k)…}` enum (9) | walk buckets | O(N log N) sort |
| 9 | pattern re-tokenize (57) | tokenized at parse | re-tokenize per match |
| 10 | `$var` read (`getsparam`) | 1 hash probe, no lock | 2 `RwLock` + clone |
| 11 | any symbol lookup | unlocked hash | global `RwLock` per access (108/36/18/15) |
| 12 | function-call frame | linked-list push/pop | `FUNCSTACK` `Mutex` push + pop |
| 13 | any `${…}`/`$(…)` expansion | reuse list, in-place bytes | fresh `LinkList` + string clone |

## Sequencing rule for the optimization phase

1. Do not pick up a backlog item until parity for its area is green — the
   parity suite is the certificate that the optimization changed only *how*, not
   *what*.
2. Change the implementation behind the faithful interface; never the contract.
   Prefer `src/extensions` or a lean rewrite of the ported function body over a
   signature change.
3. Certify with the parity suite (same pre-existing failures before/after,
   byte-for-byte match on the relevant option toggles) — the pattern cache is
   the worked example.

## Already taken from the backlog

- `${+functions/parameters[key]}` O(n²) → O(1) (subst.rs) — #1's existence path.
- Compiled-pattern cache, two-level L1/L2 (`extensions/pat_cache.rs`) — mitigates
  #3 for repeated patterns; halves p10k precmd CPU.

Both changed implementation only, certified by an unchanged parity result.
