# PORT_PLAN.md — When 100% Faithful C→Rust Is Possible, When It Isn't

This is the design rule for state-holding patterns in `src/ported/`.
PORT.md governs *what* may be ported (file/function freeze, naming).
This file governs *how* C state translates to Rust state when zsh's
single-threaded process model meets zshrs's worker pool.

The short answer: **a 1:1 port works for most state. It fails in
exactly one place — C globals that represent the shell's persistent
identity.** Those globals must become `Arc<Mutex/RwLock<…>>`, not
`thread_local!`, because subshell worker threads must share them.

---

## Progress Summary (updated 2026-05-10)

| Phase | Total Items | Done | Remaining |
|-------|-------------|------|-----------|
| Phase 1 — Bucket-1 Mutex→TLS | 16 | 0 | 16 |
| Phase 2 — Bag-of-globals structs | 20 | 16 | 4 |
| Phase 3 — Bucket-2 shared holders | 11 | 0 | 11 |
| Phase 4 — Daemon image holders | 5 | 0 | 5 |
| Phase 5 — Test invariants | 3 | 0 | 3 |

**Phase 2 remaining structs to dissolve:**
- `init.rs` `ShellState`
- `subst.rs` `SubstState`
- `modules/parameter.rs` `JobState`
- `glob.rs` `GlobState` (needs rename/field verification)

**Phase 2 landed since previous update:**
- `zle/zle_tricky.rs` `CompletionState` — deleted; the eight
  `impl Zle` methods that took `&mut CompletionState` had no real
  callers and were redundant with the live `pub fn`-at-file-scope
  C-faithful ports (`completeword` / `menucomplete` / `docomplete` /
  `docompletion`). File-scope C globals (`compcontext`, `compfunc`,
  `usemenu`, `useglob`, `nbrbeg`, `nbrend`, `origcs`, `origll`,
  `instring`, `inbackt`, `menucmp`, `comppref`, `validlist`,
  `showagain`, `lastambig`, `bashlistfirst`, `amenu`) carry the
  state.
- `zle/zle_keymap.rs` `BindState` — deleted; the C `struct bindstate`
  is only used as a stack-local in `printbinding()` /
  `scanbindings()` / `bin_bindkey -L`. Those ports model it as a
  local struct when they land.
- `zle/zle_refresh.rs` `RefreshState` — relocated to
  `src/extensions/zle_refresh_state.rs` alongside `TextAttr`,
  `RefreshElement`, `VideoBuffer`, `RegionHighlight`,
  `HighlightCategory`, `HighlightManager`. Full unification with
  C's flat `zattr u64` + `nbuf[]`/`obuf[]` arrays + discrete
  `winw`/`winh`/`vcs`/`vln`/`lpromptw`/`rpromptw` statics is a
  separate work item — entangled in the zrefresh paint loop with
  ~100+ internal sites in zle_refresh.rs to rewire.
- `zle/computil.rs` `CompState` — relocated; the bash-complete
  branch (`CompSpec`/`CompMatch`/`CompGroup`/`CompState`) moved
  to `src/extensions/bash_complete.rs`.

**Adjacent zle cascades landed (not in original Phase 2 list):**

- `zle/zle_keymap.rs` `KeymapManager` — all six fields dropped
  (`keymaps`/`current`/`current_name`/`local`/`keybuf`/`lastnamed`).
  The struct is now zero-sized; method bodies route through the
  file-scope statics that already mirror zsh's C globals
  (`keymapnamtab`, `curkeymap`, `curkeymapname`, `LOCALKEYMAP`,
  `keybuf`, `lastnamed`). `selectkeymap()` now actually writes
  `curkeymap` + `curkeymapname` (was a no-op stub matching the
  doc-comment "Without curkeymap/curkeymapname mutable globals,
  simplified"). `Zle.keymaps` field deleted; the 15 remaining
  method-call sites (`.select(X)`, `.is_emacs()`, `.is_vi_cmd()`,
  `.is_vi_insert()`, `.lookup_key(c)`) inlined to direct static
  reads / `selectkeymap()` calls.
- `zle/zle_hist.rs` `HistEntry`/`History` — relocated to
  `src/extensions/zle_history.rs` (Rust-only types; zsh's per-ZLE
  history state lives in scattered file-statics — `hist_ring`,
  `histline`, `searchstr`, `have_edits`, `hist_skip_flags`). Full
  unification with `crate::ported::hist::hist_ring` deferred.

**Zle struct field cascade (started; runs in parallel to Phase 2):**

| Field migrated | New file-scope static | C source |
|---|---|---|
| `done` | `DONE: AtomicI32` (zle_misc.rs) | `int done` zle_main.c:79 |
| `resetneeded` | `ZLE_RESET_NEEDED: AtomicI32` (zle_main.rs) | `int resetneeded` zle_main.c |
| `lastchar` | `LASTCHAR: AtomicI32` (compcore.rs) | `int lastchar` zle_main.c |
| `lastchar_wide` | `LASTCHAR_WIDE: AtomicI32` (zle_main.rs) | `int lastchar_wide` zle_main.c |
| `lastchar_wide_valid` | `LASTCHAR_WIDE_VALID: AtomicI32` (zle_main.rs) | `int lastchar_wide_valid` zle_main.c |
| `vi_last_find_char/_dir/_tail` | `VFINDCHAR/VFINDDIR/TAILADD` (zle_misc.rs) | `int vfindchar` zle_move.c:734-735 |
| `insmode` | `INSMODE: AtomicI32` (zle_main.rs) | `int insmode` zle_main.c:124 |
| `eofchar` | `EOFCHAR: AtomicI32` (zle_main.rs) | `int eofchar` zle_main.c |
| `eofsent` | `EOFSENT: AtomicI32` (zle_main.rs) | `int eofsent` zle_main.c |
| `keytimeout` | `KEYTIMEOUT: AtomicU64` (zle_main.rs) | `time_t keytimeout` zle_main.c |
| `prefixflag` | `PREFIXFLAG: AtomicI32` (zle_main.rs) | `int prefixflag` zle_main.c |
| `zle_recursive` | `ZLE_RECURSIVE: AtomicI32` (zle_main.rs) | `int zle_recursive` zle_main.c |
| `zlereadflags` | `ZLEREADFLAGS: AtomicI32` (zle_main.rs) | `int zlereadflags` zle_main.c |
| `zlecontext` | `ZLECONTEXT: AtomicI32` (zle_main.rs) | `int zlecontext` zle_main.c |
| `lastcmd` | `LASTCMD: AtomicU32` (zle_main.rs) | `int lastcmd` zle_main.c:145 |
| `incompctlfunc` | `INCOMPCTLFUNC: AtomicI32` (compctl.rs) — unified | (existing) |

**Future-wire Zle fields (kept; reserved for upcoming subsystems):**

`statusline` (zle_main.c `char *statusline`), `baud` (zle_main.c
termcap), `pre_zle_status` (zle_main.c lastval shadow), `watch_fds`
(zle_main.c `bin_zle -F` registry), `compwidget` (zle_tricky.c
new-style `compctl -K`), `hascompmod` (zle_tricky.c module guard) —
each is a real C global. **Do NOT delete these on dead-code grounds**;
they reserve the slot until their subsystem ports land.

**Zle struct fields remaining for migration (have C-global analogs):**

`zleline`/`zlecs`/`zlell` (the big three — 200+ sites each), `mark`,
`lbindk`/`bindk` (Option<Thingy>; needs Mutex<Option<Thingy>>), `zmod`
(struct), `stackhist`/`stackcs`, `vistartchange`, `undo_stack`,
`changeno`, `unget_buf` (`kungetbuf`), `vibuf` (`vibuf[36]`),
`killring`/`killringmax`, `mult`, `lastcol`, `bufstack`, `vi_chg_buf`
(`vichgbuf`), `srch_str`, `last_line`/`last_ll`/`last_cs`. Per-field
migration each carries the same rule: only if a real C global exists,
and only as a true state-of-truth unification (no dual state).

**compctl.rs Mutex count:** 37 statics still using `Mutex` (Phase 1 target: 16)

---

## Relationship to PORT.md (Read First)

PORT.md is the constitution. This file is a design supplement that
must operate inside PORT.md's hard constraints. Every action in the
checklist below stays within them:

| PORT.md rule | How this plan complies |
|---|---|
| `src/ported/` is FROZEN — no new files | All work modifies existing files in the 89-file set. |
| No new `fn` names not in `docs/zsh_c_functions.txt` | All work modifies state holders (`static …` declarations); no new `fn` introduced. |
| Rule 2: every `fn` must carry `/// Port of … from Src/…:NNNN` | All converted holders keep their existing `/// Port of file-static <c_name> from Src/<file>.c:<NNNN>` doc-comment (already present at e.g. `zle/compctl.rs:212-213, 220-221`). Same form applies to converted statics. |
| Rule 1: no abstractions without C counterpart | "Bag-of-globals" structs that aggregate file-`static`s with no matching C struct (e.g. former `MathState` at `math.rs:539`, deleted at `math.rs:599`) are explicit anti-pattern below. |
| Globals: "Mirror as static mut / Mutex<...> / thread-locals as needed for parity" | This document is the parity rubric — when each is correct vs wrong. |
| Workflow step 8: refresh `docs/port_report.html` | Each phase ends with `python3 scripts/gen_port_report.py` and `python3 scripts/match_or_warn_modules.py`. |

Conflicts between this file and PORT.md: **PORT.md wins.** If a
proposed change here would create a new file or new fn name not in
`docs/zsh_c_functions.txt`, do not make the change — raise it.

---

## Structural Fidelity Rules

### Rule S1 — Function signatures must be identical to C

Every ported function must have the same signature as its C
counterpart. If C `getjob(const char *s, const char *prog)` reads
globals internally, the Rust port is `pub fn getjob(s: &str, prog:
&str)` — NOT `pub fn getjob(s: &str, prog: &str, jobtab: &[Job],
maxjob: usize, curjob: i32, ...)`.

Do not "improve" signatures by threading state as parameters. The
globals become Rust globals (bucket 1 or 2 per above); the function
reads them the same way C does. This keeps call sites identical and
avoids signature drift that compounds across the codebase.

**Exception:** Bucket 3 (C passed pointers) are already explicit
parameters in C — those stay as parameters in Rust.

### Rule S2 — Order of code elements must match C

Within each `.rs` file, the order of:
- `static` / global declarations
- `struct` / `enum` / type definitions
- `fn` definitions
- inline comments

must match the order in the corresponding `.c` file. Reading
`jobs.rs` top-to-bottom should feel like reading `jobs.c`
top-to-bottom.

When reordering is required to fix drift, **comments must move with
their associated code**. A comment block explaining `curjob` must
stay attached to the `curjob` declaration, not be orphaned when code
moves.

---

## The Three Buckets

Every C state slot in zsh falls into one of three buckets. The Rust
port must classify each one before writing the holder.

### Bucket 1 — Per-evaluator C globals (file-`static`)

**What it is in C:** a `static` at file scope used as scratch state
for one parser/evaluator/lexer/glob-compile/completion-invocation.
zsh is single-threaded so one global suffices; conceptually it's
"the state of the thing currently being evaluated."

**Examples:** `Src/math.c` `static int unary, noeval, lastbase`,
`yyval`, `yylval`, the math operand `stack`. `Src/lex.c` lex cursor.
`Src/glob.c` pattern compile state. `Src/zle_*` ZLE per-keystroke
state.

**Rust port:** `thread_local! { static FOO: Cell/RefCell<T> }`.

**Why TLS is correct, not wrong:** in zsh C the global is per-
process. In zshrs each worker thread is its own evaluator;
per-thread is the *same* semantic, not a deviation. Two threads
each evaluating their own `$((expr))` must not corrupt each other's
operand stack — TLS gives that for free. `Cell`/`RefCell` for
interior mutability; use `Cell` for `Copy` types, `RefCell` for
owned data.

**Code stays 1:1 with C.** `unary = 1;` in C becomes
`M_UNARY.set(true);` in Rust. No call signatures change. Recursion
guards (e.g. `mathevall`'s save-to-stack-locals at `Src/math.c:387`)
port as plain Rust stack locals inside the same function — *not* as
a snapshot struct. If C doesn't have a struct, the port doesn't get
one.

### Bucket 2 — Shell-wide shared C globals

**What it is in C:** a `static` at file scope or `extern` declared in
`zsh.h` that represents the shell's persistent identity. In zsh
single-threaded these are "obviously" globals; the threading
contract is implicit because there are no other threads.

**Examples:**
- `Src/params.c` `paramtab` — the env / parameter hash table
- `Src/jobs.c` `jobtab` — background job table (read by foreground,
  mutated by SIGCHLD reaper)
- `Src/hashtable.c` `cmdnamtab`, `shfunctab`, `aliastab`, `reswdtab`,
  `emkeysmap`, `emulationstab`
- `Src/hist.c` `histlist` — the actual history list (NOT `chline`,
  the per-build cursor — that's bucket 1)
- `Src/options.c` `opts[]` — option flags (if threads can `setopt`)
- `Src/signals.c` `sigtrapped[]`, `traps[]`
- `Src/builtin.c` builtin table (read-mostly post-init)

**Rust port:** `Arc<RwLock<…>>` for read-mostly tables (paramtab,
hashtables), `Arc<Mutex<…>>` for read/write parity (jobtab),
`OnceLock<…>` for write-once-then-read (builtintab, signal trap
registry). Never `thread_local!`.

**Why TLS is wrong here:** if thread A runs `export FOO=1` and thread
B reads `$FOO`, both threads must hit the same paramtab. TLS would
silently shard the table per worker — ghost variables, missing jobs,
forgotten aliases. The bug only appears at scale; it's invisible in
single-thread tests.

**Cost of correctness:** every read takes a lock. For read-mostly
tables (`paramtab`, `aliastab`, `shfunctab`, `cmdnamtab`) use
`RwLock` so parallel readers don't serialize. For high-mutation
tables (`jobtab`, `histlist`) `Mutex` is fine.

### Bucket 3 — C passed pointer

**What it is in C:** the function takes `struct Foo *state` as a
parameter. C is already explicit about the dependency.

**Examples:** `Src/parse.c` parse-context struct, `Src/exec.c` exec
state passed through, `Src/jobs.c` functions that take
`Job *jobtab` as an argument (the table itself is bucket 2 but
many helpers receive it via pointer).

**Rust port:** Rust struct, passed as `&mut Foo` or `&Foo`.

**Code stays 1:1 with C.** Same struct fields, same call signatures,
borrow checker enforces what C only documented in comments.

---

## Decision Rule (Mechanical)

For every C state slot you encounter while porting:

1. Read the C declaration. Is it a function parameter? → **bucket 3,
   Rust struct.**
2. Is it `static` at file scope or `extern`? Read 5–10 call sites in
   the C source. Does another file (e.g. `Src/params.c` is read by
   `Src/exec.c`, `Src/builtin.c`, `Src/hist.c`) need to see *the
   same data* across what would be parallel evaluations in zshrs? →
   **bucket 2, `Arc<Lock>`.**
3. Otherwise → **bucket 1, `thread_local!`.**

If you can't decide between 1 and 2, the test is: "if two zshrs
worker threads each bumped this counter, would the result be wrong?"
- Yes (e.g. job count, env var count) → bucket 2
- No, in fact each thread *should* have its own (e.g. lex cursor,
  math eval depth) → bucket 1

---

## Where 100% Faithful Port Works

For bucket 1 and bucket 3 state, the port is mechanical and the Rust
code reads as a transliteration of the C code. This covers the
majority of `src/ported/`:

| Area | C source | Rust port |
|---|---|---|
| math eval | `Src/math.c` (file-statics) | `math.rs:334` thread_local set |
| input stack | `Src/input.c` | `input.rs:607` thread_local |
| heap arena | `Src/mem.c` | `mem.rs:111` thread_local |
| exec context | `Src/exec.c` | `context.rs:159` thread_local |
| history build cursor | `Src/hist.c chline` | `hist.rs:2624` thread_local |
| ZLE keymap state | `Src/zle_main.c` | `keymaps.rs:466` thread_local |
| fusevm bridge | (extension) | `fusevm_bridge.rs:45` thread_local |
| parser/lex/glob compile | `Src/parse.c`, `Src/lex.c`, `Src/glob.c` | TLS where ported |
| jobs operations | `Src/jobs.c` helpers | `jobs.rs` `&mut [Job]` parameter (bucket 3) |

These files preserve C structure, function names, and control flow.
PORT.md's "every line traces back to upstream C" rule is satisfied
trivially.

---

## Where 100% Faithful Port Cannot Work

For bucket 2 state, a literal C-to-Rust translation gives a
data-race-free *but semantically broken* program: each worker
thread sees its own paramtab/jobtab/histlist. The port must wrap
the table in `Arc<Lock>` even though no such wrapper exists in C.
This is the *only* sanctioned deviation from 1:1 fidelity, and it's
forced by the threading model, not by stylistic preference.

The Rust holder is still in the same `src/ported/<x>.rs` that maps
to `Src/<x>.c`. The function bodies still mirror C. Only the
*storage primitive* changes — and its name should still match the C
identifier (`PARAMTAB`, `JOBTAB`, etc.) so the trace from Rust call
site to C source remains obvious.

### Status of bucket 2 ports

| C state | Rust file | Holder primitive | Status |
|---|---|---|---|
| `paramtab` | `params.rs` | needed: `Arc<RwLock<HashMap<…>>>` | stub fns only — `params.rs:7164,7169,7189,7404` |
| `jobtab` | `jobs.rs` | currently `&mut [Job]` parameter | thread-safe by parameter; promote to `Arc<Mutex<JobTable>>` when daemon owns jobs |
| `cmdnamtab`, `shfunctab`, `aliastab`, `reswdtab` | `hashtable.rs` | needed: `Arc<RwLock<…>>` each | stub fns — `hashtable.rs:1340,1361,1409,1414` |
| `histlist` | `hist.rs` | needed: shared list; `chline`/`chwords` already `Mutex` | partial — `hist.rs:1692,1698` correct for cursors, histlist itself TBD |
| `opts[]` | `options.rs` | needed: `Arc<RwLock<[u8; OPT_SIZE]>>` if threads `setopt` | not surveyed |
| `sigtrapped`, `traps` | `signals.rs` | `OnceLock<TrapHandler>` | correct — `signals.rs:412` |
| builtin table | `builtin.rs` | `OnceLock<HashMap>` | correct — `builtin.rs:292` |

---

## Current Mismatches — Bucket 1 State Stored as `Mutex`

These sites currently use `static Mutex<…>` for state that is
logically per-evaluator (bucket 1). The Mutex is correct under
single-threaded execution but (a) needlessly serializes worker
threads doing parallel parse/completion work, and (b) drifts from
the C semantics — these were file-`static`s in C, not lock-protected
shared state. Convert to `thread_local!`.

| File:line | Symbol | Bucket-1 reason |
|---|---|---|
| `prompt.rs:1883` | `CMDSTACK` | per-evaluator parser context (case/if/for nesting) |
| `pattern.rs:1229` | `PATTERN_SCOPES` | per-pattern-compile scope stack |
| `zle/compctl.rs:214` | `CMATCHER` | per-completion-invocation matcher |
| `zle/compctl.rs:218` | `COMPCTL_TAB` | per-completion compctl set |
| `zle/compctl.rs:222` | `PATCOMPS` | per-completion pattern compls |
| `zle/compctl.rs:226` | `CCLIST` | per-completion result list mode |
| `zle/compctl.rs:230` | `SHOWMASK` | per-completion show mask |
| `zle/compctl.rs:1272` | `INCOMPFUNC` | per-completion in-function flag |
| `zle/compctl.rs:1349` | `INCOMPCTLFUNC` | per-completion in-compctl flag |
| `zle/compctl.rs:1423` | `ADDWHAT` | per-completion add-what flag |
| `zle/compctl.rs:1428` | `MATCH_LIST` | per-completion match buffer |
| `zle/compctl.rs:1698` | `PRPRE` | per-completion prefix-prefix |
| `zle/compctl.rs:1715` | `CDEPTH` | per-completion recursion depth |
| `zle/compctl.rs:1725` | `CCONT` | per-completion continuation flag |
| `zle/compctl.rs:1867` | `CMDSTR` | per-completion command string |
| `zle/compctl.rs:1921` | `CCUSED` | per-completion used set |
| `zle/compctl.rs:1990` | `WE` | per-completion word-end |
| `zle/compctl.rs:1991` | `WB` | per-completion word-begin |
| `zle/compctl.rs:1994` | `ZLEMETACS` | per-completion meta cursor |

18 conversion sites. None of these are read by other threads in C;
all are file-`static`s in `Src/zle_compctl.c` etc.

---

## Adhoc Function Marking — `// WARNING: NOT IN <FILE>.C`

PORT.md Rule 1 says "no abstractions without C counterpart" and
Rule 3 says fn names must exist in `docs/zsh_c_functions.txt`.
Reality is messier: a small number of Rust-only helpers exist
because two C callsites would each duplicate the same 15–20 lines
of save/restore or shim logic, and inlining the duplication harms
readability more than the abstraction harms fidelity.

These exceptions **must** carry a `// WARNING` block above the
function so the next porter (and `scripts/match_or_warn_modules.py`)
can see at a glance that this fn is Rust-only, why it exists, and
where the C equivalent lives.

### Required form

```rust
// WARNING: NOT IN <FILE>.C — Rust-only helper. <one-line reason>.
// C does X inside `<c_fn>()` (<file>.c:<NNNN>) — Rust factors
// it out because <concrete-justification>.
pub(crate) fn <name>(...) -> ... { ... }
```

Required fields:
- **`<FILE>.C`** — uppercase basename of the matching `Src/<file>.c`
  (e.g. `MATH.C`, `SUBST.C`, `JOBS.C`).
- **C location of the equivalent logic** — `<file>.c:<line>`. Even
  if C inlines the logic in N places, cite the canonical one.
- **Justification** — why the helper exists at all. "Two callsites"
  / "shim across rkyv mmap" / "Rust trait impl needs a free fn".
  Vague justifications ("cleaner API", "idiomatic") are rejected
  on review and the helper is deleted.

### Canonical example (`math.rs`)

```rust
// WARNING: NOT IN MATH.C — Rust-only helper. C inlines the
// xyy* save/restore directly inside `mathevall()`'s body
// (math.c:367 onward); the Rust port factors it out because two
// callsites (callmathfunc arg parsing, getmathparam indirect-string
// eval) would each duplicate ~17 lines of save/restore code.
pub(crate) fn extract_string_variables() -> HashMap<String, String> {
    ...
}
```

### Relationship to PORT.md's existing WARNING marker

PORT.md prescribes `/// WARNING: THIS IS ADHOC IMPLEMENTATION AND
NOT A FAITHFUL PORT` for fns the adhoc detector flags as having no
C counterpart at all (e.g. `modules/termcap.rs:67,92,218,231,244`).
That marker means *"this whole fn is unported placeholder code,
delete or port me."*

The shorter `// WARNING: NOT IN <FILE>.C` marker means
*"deliberate, justified Rust-only helper, do not delete."* Use the
short form when the helper is intentional and necessary; use the
long form when the fn is a stub waiting for a real port. They are
**not** interchangeable.

### Verification command

```sh
# List all fns under src/ported/ whose name is NOT in C:
rg -nE '^\s*(pub\s+)?(pub\(crate\)\s+)?fn\s+(\w+)' src/ported/ \
  | awk -F'fn ' '{print $2}' | awk '{print $1}' | tr -d '(' \
  | sort -u > /tmp/rust_fns.txt
comm -23 /tmp/rust_fns.txt docs/zsh_c_functions.txt \
  | grep -v -E '^(new|drop|fmt|clone|default|from|into|as_ref|deref|eq|hash|partial_cmp|cmp|next|poll|serialize|deserialize)$'
```

Each fn the command outputs must either get a `// WARNING:` marker
or be deleted.

---

## Anti-patterns

### 1. Bag-of-globals struct

A struct that aggregates every file-`static` from a C source file
"to thread one parameter through instead of N." This violates
PORT.md Rule 1 (no abstractions without C counterpart) and
duplicates the corresponding TLS set.

**Rule:** if C declares N file-`static`s, Rust declares N
`thread_local!` entries. No aggregation struct unless C has a
`struct foo` to point at. `math.rs:599` ("MathState struct
DELETED") is the reference for how to undo this anti-pattern when
it appears.

### 2. `Mutex` for per-evaluator state

Using `Mutex` because "it works" under single-threaded test runs.
Once worker threads run in parallel, every parser/completion
invocation contends on the same lock. The 18 sites above are this
anti-pattern.

**Rule:** if the C source has it as a file-`static`, the Rust port
has it as `thread_local!`. `Mutex` is reserved for bucket 2.

### 3. `thread_local!` for shared tables

Putting `paramtab` or `jobtab` in TLS because "it's a global in C."
This silently per-shards the shell's identity. The data race goes
away (each thread has its own copy) but the program is wrong.

**Rule:** read 5+ call sites in C across multiple files. If two
different evaluation contexts must see the same data, it's bucket
2 — `Arc<Lock>`, not TLS.

---

## Conversion Plan — Checklist

Order is set by blast radius. Tick each item only when the change
is committed and `cargo build` is green.

**Per-item discipline (PORT.md workflow):**

1. Read the C source line cited in the bullet (`Src/<file>.c:<NNNN>`).
2. Verify the holder name matches the C identifier byte-for-byte
   (uppercased to Rust `static`/`thread_local!` convention).
3. Make the change. No new `fn`, no new file.
4. Update or keep the `/// Port of file-static <c_name> from
   Src/<file>.c:<NNNN>` doc-comment on the holder.
5. `cargo build --lib && cargo test --lib -- <module>` (targeted,
   not full suite — per global preferences).
6. Commit citing the C line in the message body.

**End-of-phase discipline (PORT.md workflow step 8):**

- `python3 scripts/gen_port_report.py` — refresh
  `docs/port_report.html`.
- `python3 scripts/match_or_warn_modules.py` — verify no new adhoc
  WARNINGs were introduced.

### Phase 1 — Mechanical bucket-1 fixes (no semantic change)

Convert `static Mutex<…>` to `thread_local!` for state that is
per-evaluator in C. Each line includes the file:line of the current
Mutex declaration plus the C source citation. Each converted holder
**must keep** its `/// Port of file-static <c_name> from
Src/<file>.c:<NNNN>` doc-comment per PORT.md Rule 2 (the citation
form already in place — see e.g. `zle/compctl.rs:212-213`).

PORT.md compliance: no new files, no new `fn` names. Only the
*holder primitive* changes (`Mutex` → `thread_local!`). Keep the
SCREAMING_SNAKE name verbatim from the C identifier.

**Verified bucket-1 (file-`static` in C, per-completion-call /
per-evaluator):**

- [ ] `prompt.rs:1883` `CMDSTACK` ← `Src/init.c` cmdstack (parser
      context stack)
- [ ] `pattern.rs:1229` `PATTERN_SCOPES` ← `Src/pattern.c` per-
      pattern-compile scopes
- [ ] `zle/compctl.rs:226` `CCLIST` ← `Src/Zle/compctl.c:63`
      `static int cclist`
- [ ] `zle/compctl.rs:230` `SHOWMASK` ← `Src/Zle/compctl.c:66`
      `static unsigned long showmask`
- [ ] `zle/compctl.rs:1272` `INCOMPFUNC` ← `Src/Zle/compctl.c`
      `int incompfunc`
- [ ] `zle/compctl.rs:1349` `INCOMPCTLFUNC` ← `Src/Zle/compctl.c`
      `int incompctlfunc`
- [ ] `zle/compctl.rs:1423` `ADDWHAT` ← `Src/Zle/compctl.c:1749`
      `static int addwhat`
- [ ] `zle/compctl.rs:1428` `MATCH_LIST` ← `Src/Zle/compctl.c`
      per-call match heap
- [ ] `zle/compctl.rs:1698` `PRPRE` ← `Src/Zle/compctl.c` per-call
      prefix-prefix
- [ ] `zle/compctl.rs:1715` `CDEPTH` ← `Src/Zle/compctl.c` per-call
      depth
- [ ] `zle/compctl.rs:1725` `CCONT` ← `Src/Zle/compctl.c` per-call
      continuation
- [ ] `zle/compctl.rs:1867` `CMDSTR` ← `Src/Zle/compctl.c` per-call
      command string
- [ ] `zle/compctl.rs:1921` `CCUSED` ← `Src/Zle/compctl.c` per-call
      used set
- [ ] `zle/compctl.rs:1990` `WE` ← `Src/Zle/compctl.c` per-call
      word-end
- [ ] `zle/compctl.rs:1991` `WB` ← `Src/Zle/compctl.c` per-call
      word-begin
- [ ] `zle/compctl.rs:1994` `ZLEMETACS` ← `Src/Zle/compctl.c` per-
      call meta cursor

**Reclassified to bucket 2 (do NOT move to TLS — they are user-
registered shared registries, not per-call scratch). Verified
against `Src/Zle/compctl.c`:**

- `zle/compctl.rs:214` `CMATCHER` — `Src/Zle/compctl.c:36
  static Cmlist cmatcher`. *File-static, but written by `compctl
  -M` (line 326-327: `freecmlist(cmatcher); cmatcher = cpcmlist(l);`)
  and read by every completion call.* User-registered matcher
  registry → bucket 2. Keep `Mutex` (or promote to `RwLock` for
  parallel reads).
- `zle/compctl.rs:218` `COMPCTL_TAB` — `Src/Zle/compctl.c:46
  HashTable compctltab`. *No `static` — extern global declared in
  zle.h, populated by `compctl name args`, read by every
  completion call.* User registry → bucket 2. Keep `Mutex`.
- `zle/compctl.rs:222` `PATCOMPS` — `Src/Zle/compctl.c:51
  Patcomp patcomps`. *No `static` — extern global, list of
  pattern compctls registered via `compctl -p`.* User registry →
  bucket 2. Keep `Mutex`.

These three move to Phase 3 (bucket-2 holders) — promote to
`Arc<RwLock<…>>` when paramtab/hashtables get the same treatment.

**End of Phase 1:**

- [ ] `python3 scripts/gen_port_report.py`
- [ ] `python3 scripts/match_or_warn_modules.py` — zero new
      WARNINGs
- [ ] `cargo build --lib` green
- [ ] `cargo test --lib -- compctl prompt pattern` green

### Phase 2 — Bucket audit of remaining `*State` structs

For each struct: read the matching `Src/<file>.c`, classify, fix or
keep. Cite C `file:line` in the commit message. Per PORT.md Rule 1,
a `*State` struct is legal **only if** it mirrors a real `struct`
in the matching `Src/<file>.c` — otherwise it's a bag-of-globals
abstraction and must be deleted in favor of `thread_local!` for
each field.

Verdict for each struct will be one of:
- **bucket 1** (delete struct, fields → `thread_local!` set)
- **bucket 3** (keep struct, verify field-for-field with C `struct`)
- **rename** (struct exists in C but under a different name)

**Audit verdict** for each `*State`. Counted C structs in each
matching `Src/<file>.c` via:
```sh
grep -cE '^(typedef )?struct \w+\s*\{' Src/<file>.c
```
Files that report 0 are bucket-1 (anti-pattern; dissolve to
`thread_local!` set). Files with ≥1 struct need field-by-field
verification: bucket-3 if it matches a real C struct, rename if a
real C struct exists under a different name.

**Bucket 1 — anti-pattern bag-of-globals (dissolve to `thread_local!`):**

- [ ] `init.rs:36` `ShellState` ← `Src/init.c` has 0 structs;
      aggregates `argv0`, `argzero`, `posixzero`, `mypid`, `ppid`,
      `shtty`, `lineno`, `path`, `fpath`, etc. — all individual
      C globals. Verdict: **bucket-1**, dissolve. Blast radius:
      moderate (used by `init_io`, `setupvals`, `clone.rs`).
      **Status: NOT DONE — struct still exists.**
- [ ] `subst.rs:201` `SubstState` ← `Src/subst.c` has 0 structs.
      Verdict: **bucket-1**. Blast radius: large (every subst
      caller). **Status: NOT DONE — struct still exists.**
- [x] `loop.rs:40` `LoopState` ← `Src/loop.c` has 0 structs.
      Verdict: **bucket-1**. **Status: DONE — struct dissolved.**
- [x] `loop.rs:237` `CForState` ← same. Verdict: **bucket-1**.
      **Status: DONE — struct dissolved.**
- [x] `loop.rs:259` `TryState` ← same. Verdict: **bucket-1**.
      **Status: DONE — struct dissolved.**
- [x] `zle/zle_vi.rs:22` `ViState` ← `Src/Zle/zle_vi.c` has 0
      structs. Verdict: **bucket-1**. **Status: DONE — struct dissolved.**
- [x] `zle/zle_tricky.rs:17` `CompletionState` ←
      `Src/Zle/zle_tricky.c` has 0 structs. Verdict: **bucket-1**.
      **Status: DONE — struct deleted with the eight `impl Zle`
      methods that took `&mut CompletionState`. State carried by
      file-scope C globals (USEMENU, USEGLOB, WOULDINSTAB, NBRBEG,
      NBREND, ORIGCS, ORIGLL, INSUBSCR, INSTRING, INBACKT,
      ORIGLINE, LASTPREBR, LASTPOSTBR, COMPQUOTE, AUTOQ, MENUCMP,
      COMPPREF, VALIDLIST, SHOWAGAIN, LASTAMBIG, BASHLISTFIRST,
      AMENU) already present in zle_tricky.rs.**
- [x] `zle/compcore.rs:26` `CompState` ← `Src/Zle/compcore.c` has
      0 structs. Verdict: **bucket-1** (also disambiguate from
      computil's CompState — see below). **Status: DONE — struct dissolved.**
- [x] `modules/random.rs:17` `RandomState` ← `Src/Modules/random.c`
      has 0 structs (only `static uint32_t rand_buff[8]` and
      `static int buf_cnt` at lines 50-51). Verdict: **bucket-1**;
      two `thread_local!`s mirror the C statics directly.
      **Status: DONE — struct dissolved.**
- [x] `modules/socket.rs:355` `UnixSocketState` ← 0 structs.
      Verdict: **bucket-1**. **Status: DONE — struct dissolved.**
- [x] `modules/watch.rs:64` `WatchState` ← 0 structs. Verdict:
      **bucket-1**. **Status: DONE — struct dissolved.**
- [x] `modules/pcre.rs:18` `PcreState` ← 0 structs. Verdict:
      **bucket-1**. **Status: DONE — struct dissolved.**

**Bucket 3 — real C struct exists; verify field-for-field or rename:**

- [ ] `glob.rs:462` `GlobState` ← `Src/glob.c:168` has `struct
      globdata` (15 fields including `gd_pathpos`, `gd_pathbuf`,
      `gd_matchct`, `gd_quals`, etc.). Rust struct has the same
      role (per-glob state) and overlapping fields (`pathbuf`,
      `pathpos`, `matches`, `qualifiers`). Verdict: **rename**
      to `GlobData` (or keep alias) + verify field parity.
      **Status: NOT DONE — needs field parity verification.**
- [x] `keymaps.rs:11` `ZleState` ← `Src/Zle/zle_main.c:432`
      `struct ztmout` and `:1927 struct findfunc` — neither
      matches ZleState's role (top-level ZLE state aggregator).
      Verdict: **bucket-1**, dissolve into per-static
      `thread_local!`s. **Status: DONE — struct dissolved.**
- [x] `zle/zle_keymap.rs:65` `BindState` ← `Src/Zle/zle_keymap.c`
      has 5 structs (`keymapname`, `keymap`, `key`, etc.). None
      match BindState (binding-context aggregator). Verdict:
      **bucket-1**, dissolve. **Status: DONE — struct + flags
      deleted (zle_keymap.rs:133 comment cites the rationale: C
      `struct bindstate` is only used as a stack-local in
      `printbinding()`/`scanbindings()`/`bin_bindkey -L`; those
      ports model it as a local struct when they land).**
- [x] `zle/zle_utils.rs:166` `UndoState` ← `Src/Zle/zle_utils.c`
      has 2 structs (`zle_region`, `zle_position`). Neither maps
      to UndoState — C has `static struct change *changes` /
      `static struct change *curchange` as file-statics for the
      undo ring. Verdict: **bucket-1**, dissolve.
      **Status: DONE — struct dissolved.**
- [x] `zle/zle_refresh.rs:156` `RefreshState` ←
      `Src/Zle/zle_refresh.c:815` `struct rparams`. Field-by-
      field verification needed; if matches, **rename** to
      `RParams`. Otherwise **bucket-1**.
      **Status: DONE (partial) — moved to
      `src/extensions/zle_refresh_state.rs` along with TextAttr /
      RefreshElement / VideoBuffer / RegionHighlight /
      HighlightCategory / HighlightManager (all Rust-only
      abstractions over zsh's `zattr u64` + `nbuf[]`/`obuf[]` flat
      arrays + discrete winw/winh/vcs/vln/lpromptw/rpromptw
      statics). The standalone `pub struct rparams` lower in
      zle_refresh.rs (the legit port of c:815) is unaffected. Full
      unification to the C flat-array model deferred — ~100+
      internal sites in zle_refresh.rs to rewire.**
- [x] `zle/computil.rs:542` `CompState` ← `Src/Zle/computil.c`
      has 13 structs (`cdstate`, `cdstr`, `cdrun`, etc.). The
      most likely match is `cdstate` at line 40. Verdict:
      **rename** to `Cdstate` + verify fields.
      **Status: DONE — the bash-complete CompState branch (and
      its companions CompSpec/CompMatch/CompGroup) relocated to
      `src/extensions/bash_complete.rs`. The legit
      C-faithful `cdstate` port (matching computil.c:40 with all
      15 fields) lives in computil.rs alongside 12 other ported
      C structs (caarg, caopt, cadef, castate, cvdef, cvval,
      cvstate, cdstr, cdrun, cdset, ctags, ctset).**
- [x] `modules/zpty.rs:629` `ZptyState` ←
      `Src/Modules/zpty.c:48 struct ptycmd` — single struct in
      the file. Verdict: **rename** to `Ptycmd` + verify fields.
      **Status: DONE — struct dissolved.**
- [ ] `modules/parameter.rs:704` `JobState` ←
      `Src/Modules/parameter.c:2179 struct pardef`. Pardef is
      the parameter-definition table for module-exported
      parameters; JobState is a magic-`$jobstates`-array
      synthesizer. Different roles. Verdict: **bucket-1** (if
      JobState aggregates file-statics) or **rename** if it
      mirrors a different C struct (jobstate_t analog in
      jobs.c). **Status: NOT DONE — struct still exists.**

**Per-struct dissolution work** is one commit each — bucket-1
verdicts above are blast-radius-ordered: smallest (single-module
modules/*) first, largest (init/subst/loop) last. Each
dissolution must keep the surrounding `fn`-name set unchanged
(PORT.md compliance — bucket-1 conversion changes storage, not
identity).

**End of Phase 2:**

- [ ] `python3 scripts/gen_port_report.py`
- [ ] `python3 scripts/match_or_warn_modules.py` — zero new
      WARNINGs; any deletions matched by removal of duplicate
      adhoc fns
- [ ] `cargo build --lib` green
- [ ] Affected test modules green (no full-suite run)

### Phase 3 — Bucket-2 holders (as underlying ports land)

PORT.md compliance: holders live in the existing `src/ported/<x>.rs`
that maps to `Src/<x>.c`. No new files. The Rust holder name
matches the C identifier (`PARAMTAB` ← `paramtab`, etc.). Per
PORT.md "Mirror globals as static mut / Mutex<...> / thread-locals
as needed for parity, not Rust elegance" — `Arc<RwLock>` is the
parity-preserving choice when threading forces shared mutation.

- [ ] `params.rs` `PARAMTAB` → `Arc<RwLock<HashMap<String, Param>>>`
      ← `Src/params.c paramtab` (replaces stub fns at
      `params.rs:7164,7169,7189,7404`)
- [ ] `hashtable.rs` `CMDNAMTAB` → `Arc<RwLock<…>>`
      ← `Src/hashtable.c cmdnamtab` (replaces stub at
      `hashtable.rs:1340`)
- [ ] `hashtable.rs` `SHFUNCTAB` → `Arc<RwLock<…>>`
      ← `Src/hashtable.c shfunctab` (`hashtable.rs:1361`)
- [ ] `hashtable.rs` `ALIASTAB` → `Arc<RwLock<…>>`
      ← `Src/hashtable.c aliastab` (`hashtable.rs:1409`)
- [ ] `hashtable.rs` `RESWDTAB` → `Arc<RwLock<…>>`
      ← `Src/hashtable.c reswdtab`
- [ ] `hist.rs` `HISTLIST` → `Arc<Mutex<…>>` (shared history list);
      `CHLINE`/`CHWORDS` already `Mutex` at `hist.rs:1692,1698` —
      keep (they're shared per-line accumulators)
- [ ] `options.rs` `OPTS` → `Arc<RwLock<[u8; OPT_SIZE]>>`
      ← `Src/options.c opts[]` (only if worker threads can `setopt`
      independently — read 5+ call sites in `Src/options.c` first)
- [ ] `jobs.rs` `JOBTAB` → `Arc<Mutex<JobTable>>`
      ← `Src/jobs.c jobtab`. Currently passed as `&mut [Job]`
      (`jobs.rs:846,985,1054,…`) — promote when daemon owns jobs
      reaping
- [ ] `zle/compctl.rs:214` `CMATCHER` keep `Mutex`, consider
      `Arc<RwLock<…>>` ← `Src/Zle/compctl.c:36 static Cmlist
      cmatcher` (user-registered via `compctl -M`)
- [ ] `zle/compctl.rs:218` `COMPCTL_TAB` keep `Mutex`, consider
      `Arc<RwLock<…>>` ← `Src/Zle/compctl.c:46 HashTable
      compctltab` (user-registered via `compctl name args`)
- [ ] `zle/compctl.rs:222` `PATCOMPS` keep `Mutex`, consider
      `Arc<RwLock<…>>` ← `Src/Zle/compctl.c:51 Patcomp patcomps`
      (user-registered via `compctl -p`)

**End of Phase 3:**

- [ ] `python3 scripts/gen_port_report.py`
- [ ] `python3 scripts/match_or_warn_modules.py` — zero new
      WARNINGs
- [ ] `cargo build --lib` green
- [ ] `cargo test --lib -- params jobs hashtable hist options
      compctl` green
- [ ] `tests/shared_state_visible.rs` exists and passes (added in
      Phase 5 — but include the test as part of Phase 3 acceptance
      since these are the holders it guards)

### Phase 4 — Daemon image holders

Per `cache_architecture_rkyv.md`. Daemon code lives in
`src/extensions/` (zsh has no daemon — this is a sanctioned PORT.md
Rule 1 exception #1, "features that zsh C does not have"). The
ported holder in `src/ported/<x>.rs` reads from a daemon-owned
mmap via a thin bridge in `src/extensions/`. Only after the daemon
ships.

- [ ] `paramtab` daemon image: `Arc<RwLock<&'static rkyv::Archived<ParamTable>>>`
      mmapped from daemon catalog
- [ ] `cmdnamtab` daemon image (PATH hash): same pattern
- [ ] `shfunctab` daemon image (autoload registry): same pattern
- [ ] `aliastab` daemon image: same pattern
- [ ] Lock protects the mmap pointer swap on daemon rebuild
      notification — readers hold the lock for the duration of one
      lookup only

**End of Phase 4:**

- [ ] `python3 scripts/gen_port_report.py`
- [ ] `cargo build --lib` green
- [ ] Daemon-bridge tests pass

### Phase 5 — Test invariants

PORT.md Rule 3 exemption: tests are exempt from the C-name rule
but "must still describe what C behavior they verify." Both test
files below describe the bucket invariant they guard.

- [ ] `tests/shared_state_visible.rs` — N worker threads, each
      mutates a unique key in paramtab/aliastab/jobtab/histlist,
      observer thread reads back all N mutations. Fails if any
      bucket-2 holder gets demoted to TLS. **Verifies:** zsh C
      single-process semantics for `Src/params.c paramtab`,
      `Src/hashtable.c aliastab`, `Src/jobs.c jobtab`,
      `Src/hist.c histlist` are preserved across worker threads.
- [ ] `tests/per_evaluator_isolation.rs` — N worker threads each
      run a `$((expr))` with side-effecting variables; verifies no
      cross-thread leak through the math TLS set. **Verifies:**
      `Src/math.c` file-statics (`unary, noeval, lastbase, yyval,
      yylval, stack`) behave per-evaluator under threading, as
      they do per-process in C.
- [ ] Both test files added to `tree_walker_absent.rs`-style
      invariant guard list (per CLAUDE.md "96-test invariant is
      load-bearing"). Update `tests/no_tree_walker_dispatch.rs`
      sibling list with these names.

**End of Phase 5:**

- [ ] `cargo test --lib -- shared_state_visible
      per_evaluator_isolation` green
- [ ] `python3 scripts/gen_port_report.py`

---

## Quick-Reference Decision Card

```
C declaration            →  Rust holder
─────────────────────────────────────────────────────────
fn foo(struct X *st)     →  fn foo(st: &mut X)        (bucket 3)
static int x; (file)     →  thread_local! Cell<i32>   (bucket 1)
static struct htab *t;   →  Arc<RwLock<HashMap>>      (bucket 2)
extern T t; (in zsh.h)   →  Arc<RwLock/Mutex<T>>      (bucket 2)
```

If unsure between bucket 1 and bucket 2: read 5+ C call sites
across files. Two evaluation contexts needing the same data → 2.
Each context wanting its own copy → 1.

---

## Test Invariants (Bucket 2 Specifically)

The `tree_walker_absent.rs` and `no_tree_walker_dispatch.rs` tests
guard architectural decay generally. Bucket 2 needs its own
behavioral pin: a multi-threaded test that spawns N workers, has
each one mutate the shared table (set a unique env var, register a
unique alias, append a history entry), then verifies all N
mutations are visible from a single observer thread. If TLS ever
sneaks back in for a bucket-2 holder, this test fails. Add
`tests/shared_state_visible.rs` when the first bucket-2 holder
goes live.
