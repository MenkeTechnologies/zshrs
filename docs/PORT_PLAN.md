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

## Progress Summary (updated 2026-10-27, v0.13.56)

| Phase | Total Items | Done | Remaining |
|-------|-------------|------|-----------|
| Phase 1 — Bucket-1 Mutex→TLS | 16 | 16 | 0 |
| Phase 2 — Bag-of-globals structs | 20 | 18 | 2 (`RefreshState` family, `HistEntry`/`History` — both back in `src/ported/`) |
| Phase 3 — Bucket-2 shared holders | 11 | 11 | 0 |
| Phase 4 — Daemon image holders | 5 | 0 | 5 |
| Phase 5 — Test invariants | 3 | 3 | 0 |
| Phase 6 — ShellExecutor field dissolution | 61 at extraction (`a8353f17d4`) | 43 of the 61 gone (29 fields added since) | 47 fields (as of 2026-10-27, v0.13.56) |

**Phase 2 remaining structs to dissolve:** `zle/zle_refresh.rs`
`RefreshState` family and `zle/zle_hist.rs` `HistEntry`/`History`
(relocation to `src/extensions/` reverted in `489ebe26f3`; see Phase 2).

**Phase 6 in flight:** `pub struct ShellExecutor` lives in
`src/vm_helper.rs` (was `src/exec.rs`, which no longer exists); field
count is derived by `awk '/^pub struct ShellExecutor/,/^}/'
src/vm_helper.rs | grep -cE '^\s*(pub )?[a-z_]+:[^:]'` (47 as of
2026-10-27, v0.13.56; the `[^:]` keeps `std::…` continuation lines of
multi-line field types out of the count). Each remaining field is
audited against the canonical zsh C global that callers should reach
directly. Deletions (newer→older): `zwc_cache`, `loop_signal`,
`autoload_pending`, `options`, `cmd_stack`, `hook_functions`,
`command_hash`, `named_dirs`, `readonly_vars`, `dir_stack`,
`last_subst`/`sub_flags`/`in_paramsubst_nest`/`zftp`/`style_table`
(5 in one commit), `expanding_aliases`. Routes through `paramtab`,
`cmdnamtab`, `OPTS_LIVE`, `prompt::CMDSTACK`, `aliastab`,
`BREAKS`/`CONTFLAG`.
Same anti-pattern as the dissolved `Zle` struct (Rule 1: bag-of-
globals with no `struct executor` in C).

**Phase 2 final-batch dissolutions (verified 2026-05-12):**
- `init.rs` `ShellState` — verified absent via `grep -rn 'struct
  ShellState' src/`. The init.c file-statics live directly in
  `init.rs` (`noexitct` … `use_exit_printed`) as individual `static AtomicI32/AtomicUsize/
  Mutex<...>` mirrors of `noexitct`, `zunderscore`, `underscorelen`,
  `underscoreused`, `sourcelevel`, `SHTTY`, `shout`, `tcstr`,
  `tclen`, `tclines`, `tccolumns`, `hasam`, `hasxn`, `tccolours`,
  `zshhooks`, `argv0`, `zle_entry_ptr`, `zle_load_state`,
  `compctlreadptr`, `use_exit_printed`. No aggregator struct.
- `subst.rs` `SubstState` — verified absent via grep.
- `modules/parameter.rs` `JobState` — verified absent via grep.
  (The `JobState` enum in `src/exec_jobs.rs` is a separate type
  modeling job lifecycle states Running/Stopped/Done — not the
  bag-of-globals JobState the plan called out.)
- `glob.rs` `GlobState` — now `glob.rs` `globdata` (the interim
  `GlobData` name was dissolved in `ac53b7b696`), port of
  `Src/glob.c:168 struct globdata`. Field parity is partial:
  `gd_matchsz`/`gd_matchbuf`/`gd_matchptr` collapse into
  `matches: Vec<gmatch>`; `matchct`, `pathbuf`/`pathpos`/`pathbufsz`/
  `pathbufcwd`, `gf_nullglob`/`gf_markdirs`/`gf_noglobdots`/
  `gf_listtypes`/`gf_pre_words`/`gf_post_words` are fields. The
  doc-comment above `globdata` notes the collapses.

**Phase 2 landed since previous update:**
- `zle/zle_tricky.rs` `CompletionState` — deleted; the eight
  `impl Zle` methods that took `&mut CompletionState` had no real
  callers and were redundant with the live `pub fn`-at-file-scope
  C-faithful ports (`completeword` / `menucomplete` / `docomplete` /
  `docompletion`). File-scope C globals (`compcontext` in
  `complete.rs`, `compfunc` in `compcore.rs`, and in `zle_tricky.rs`
  `usemenu`, `useglob`, `nbrbeg`, `nbrend`, `origcs`, `origll`,
  `instring`, `inbackt`, `menucmp`, `comppref`, `validlist`,
  `showagain`, `lastambig`, `bashlistfirst`, `amenu`, `lincmd`)
  carry the state.

**Tab dispatch chain fully wired (zle_tricky / complete):**

- `expandorcomplete` (the default `^I` binding per `EMACSBIND[9]`
  / `VIINSBIND[9]` mirroring `zle_bindings.c:97,265`) → `docomplete(
  COMP_EXPAND_COMPLETE)` → real 3-way switch on `lst` in
  `zle_tricky.rs::docomplete` mirroring C `zle_tricky.c:817-870`:
  `COMP_SPELL` → `spckword` path, `COMP_ISEXPAND` → `doexpansion`
  with `COMP_EXPAND_COMPLETE` fall-through to `docompletion`, else
  → `docompletion`. Earlier versions short-circuited the entire
  dispatch to `do_completion`, leaving `doexpansion` unreachable.
- `doexpansion(s, lst, olst, explincmd)` and `docompletion(s, lst,
  incmd)` are now real ports (was 1-arg / 0-arg stubs).
- `lincmd` is a file-scope `AtomicI32` `zle_tricky.rs` `LINCMD`
  (mirrors C `c:139`), set by `get_comp_string` from the lexer's
  `incmdpos()` per C `c:1160` and the `c:1198-1202` command-position
  expression (`inredir()`/`incmdpos()`/`incond()`), landed in
  `d54efb42be`. Threaded through `docomplete` → `compldat.incmd`
  per C `c:805`.
- All six C COMPLETEHOOK Hookfns from `complete.c:1762-1767` are
  registered in `complete.rs::boot_`: `complete` → `do_completion`,
  `before_complete` → `before_complete`, `after_complete` →
  `after_complete`, `list_matches` → `list_matches`,
  `invalidate_list` → `invalidate_list` via `(Hookdef, void*) -> i32`
  thunks, and `accept_completion` → `accept_last` via a `transmute`
  of `fn() -> i32` to `Hookfn`, mirroring C's `(Hookfn) accept_last`
  cast (`8530797a0f`). User-installed `complete` Hookfns fire.
- `zle/zle_keymap.rs` `BindState` — fake deleted (`d20c5a14bd`); the
  C-faithful `struct bindstate` (`Src/Zle/zle_keymap.c:95-104`) is
  ported as `zle_keymap.rs` `bindstate` and used by `scanbindlist` /
  `bindlistout`.
- `zle/zle_refresh.rs` `RefreshState` — **NOT relocated.** It was moved
  to `src/extensions/zle_refresh_state.rs` in `cc2b57d31d`, then moved
  back in `489ebe26f3`; `RefreshState`, `TextAttr`, `VideoBuffer`,
  `HighlightManager` live in `src/ported/zle/zle_refresh.rs` again.
  The C-faithful `rparams` (`Src/Zle/zle_refresh.c:815`) is also in
  `zle_refresh.rs`. Full unification with C's flat `zattr u64` +
  `nbuf[]`/`obuf[]` arrays + discrete `winw`/`winh`/`vcs`/`vln`/
  `lpromptw`/`rpromptw` statics is a separate work item, entangled
  in the zrefresh paint loop.
- `zle/computil.rs` `CompState` — relocated; the bash-complete
  branch (`CompSpec`/`CompMatch`/`CompGroup`/`CompState`) moved
  to `src/extensions/bash_complete.rs`.

**Adjacent zle cascades landed (not in original Phase 2 list):**

- `zle/zle_keymap.rs` `KeymapManager` — deleted entirely
  (`79c34d3e66`: unit struct and its 32-method impl → 3 free fns).
  Its six fields (`keymaps`/`current`/`current_name`/`local`/`keybuf`/
  `lastnamed`) were dropped first; callers route through the
  file-scope statics that mirror zsh's C globals
  (`keymapnamtab`, `curkeymap`, `curkeymapname`, `LOCALKEYMAP`,
  `keybuf`, `lastnamed`). `selectkeymap()` now actually writes
  `curkeymap` + `curkeymapname` (was a no-op stub matching the
  doc-comment "Without curkeymap/curkeymapname mutable globals,
  simplified"). `Zle.keymaps` field deleted; the 15 remaining
  method-call sites (`.select(X)`, `.is_emacs()`, `.is_vi_cmd()`,
  `.is_vi_insert()`, `.lookup_key(c)`) inlined to direct static
  reads / `selectkeymap()` calls.
- `zle/zle_hist.rs` `HistEntry`/`History` — **NOT relocated.** Moved
  to `src/extensions/zle_history.rs` in `6afa290d90`, then moved back
  in `489ebe26f3`; both Rust-only types live in
  `src/ported/zle/zle_hist.rs` again (zsh's per-ZLE history state
  lives in scattered file-statics — `hist_ring`, `histline`,
  `searchstr`, `have_edits`, `hist_skip_flags`). Full unification
  with `crate::ported::hist::hist_ring` deferred.

**Zle struct cascade — COMPLETE (2026-05-12, 5 commits):**

The field-bearing `pub struct Zle { ~50 fields }` is fully dissolved.
All fields migrated to file-scope statics matching the C source's
file-`static`s (bucket 1 per the rule above). Methods hoisted to
free fns. `pub struct Zle;` unit marker and `impl Default for Zle`
deleted. `Zle::new()` replaced with free `zle_reset()`. All 556 zle
tests passed under both serial and parallel cargo test (2026-05-12).

Hashes below are the post-rewrite commits on `main`; the original
hashes recorded here no longer resolve.

| Phase | Commit | What landed |
|---|---|---|
| Field migration | `800523595d` | 50 statics added, ~4,100 call sites rewritten |
| Method hoist | `cf7c9ca8dc` | 213 impl-Zle methods → free fns; 42 duplicates deduped |
| Test compile | `3a3359624f` | restored real bodies for uplineorhistory/downlineorhistory |
| Unit-marker delete | `32433a1a4a` | `pub struct Zle;` + `impl Default` + `Zle::new()` (193 callers) |
| Test serialise + OOB fix | `5ef5bc1559` | `zle_test_setup()` helper, 19 OOB loop reorders, vi_yank_whole_line N-line fix |

The full static list (all 50 + the 16 already-migrated): `ZLELINE`,
`ZLECS`, `ZLELL`, `MARK`, `LBINDK`, `BINDK`, `ZMOD`, `STATUSLINE`,
`STACKHIST`, `STACKCS`, `VISTARTCHANGE`, `UNDO_STACK`, `CHANGENO`,
`KUNGETBUF`, `BAUD`, `COMPWIDGET`, `HASCOMPMOD`, `TTYFD`, `LPROMPT`,
`RPROMPT`, `PRE_ZLE_STATUS`, `VIBUF`, `KILLRING`, `KILLRINGMAX`,
`YANKLAST`, `NEG_ARG`, `MULT`, `HISTORY`, `LASTCOL`, `BUFSTACK`,
`VICHGBUF`, `SRCH_STR`, `LASTLINE`, `LASTLL`, `LASTCS`, `CURCHANGE`,
`UNDO_CHANGENO`, `UNDO_LIMITNO`, `VIINSBEGIN`, `YANKB`, `YANKE`,
`YANKCS`, `KCT`, `VIMARKS`, `REGION_ACTIVE`, `PENDING_HOOKS`,
`RAW_LP`, `RAW_RP`, `HIGHLIGHT` — plus the prior batch
(`DONE`/`ZLE_RESET_NEEDED`/`LASTCHAR`/`LASTCHAR_WIDE`/
`LASTCHAR_WIDE_VALID`/`VFINDCHAR`/`VFINDDIR`/`TAILADD`/`INSMODE`/
`EOFCHAR`/`EOFSENT`/`KEYTIMEOUT`/`PREFIXFLAG`/`ZLE_RECURSIVE`/
`ZLEREADFLAGS`/`ZLECONTEXT`/`LASTCMD`/`INCOMPCTLFUNC`). Every static
carries a `/// Port of <C decl> from Src/Zle/<file>.c:<line>` doc
comment per PORT.md Rule 2.

Test serialisation: `ZLE_TEST_LOCK: Mutex<()>` + `zle_test_setup()`
helper in zle_main.rs. Tests acquire the guard for their body's
lifetime so cargo's parallel runner effectively serialises ZLE-
touching tests. No new external dep (no `serial_test` crate). Both
helpers allowlisted with WARNING-form rationale (C is single-
threaded, no C counterpart).

**compctl.rs Mutex holders:** count with
`rg -c '^\s*(pub(\(crate\))? )?static \w+: (std::sync::)?Mutex<'
src/ported/zle/compctl.rs` (20 as of 2026-10-27, v0.13.56). The
bucket-1 statics in the Phase 1 table are `thread_local!` (or were
deduped into other files — see Phase 1); bucket-2 `CMATCHER`/
`COMPCTL_TAB`/`PATCOMPS` are `RwLock`. The `Mutex` statics still in
compctl.rs — `COMPQIPREFIX`, `COMPQISUFFIX`, `COMPISUFFIX`, `ISUF`,
`CLWORDS`, `CLWSIZE`, `CLWNUM`, `CLWPOS`, `BRANGE`, `ERANGE`,
`LINWHAT`, `LINREDIR`, `INSUBSCR`, `CC_COMPOS`/`CC_DEFAULT`/
`CC_FIRST`/`CC_DUMMY`, `LASTCCUSED`, `COMPCTLREAD_INSTALLED` — have
not been re-classified yet. Re-classification is bucket-by-bucket
follow-up work, not blocking.

---

## Relationship to PORT.md (Read First)

PORT.md is the constitution. This file is a design supplement that
must operate inside PORT.md's hard constraints. Every action in the
checklist below stays within them:

| PORT.md rule | How this plan complies |
|---|---|
| `src/ported/` is FROZEN — no new files | All work modifies existing files in the 106-file set. |
| No new `fn` names not in `docs/zsh_c_functions.txt` | All work modifies state holders (`static …` declarations); no new `fn` introduced. |
| Rule 2: every `fn` must carry `/// Port of … from Src/…:NNNN` | All converted holders keep their existing `/// Port of file-static <c_name> from Src/<file>.c:<NNNN>` doc-comment (already present at e.g. `zle/compctl.rs` `CMATCHER`, `COMPCTL_TAB`). Same form applies to converted statics. |
| Rule 1: no abstractions without C counterpart | "Bag-of-globals" structs that aggregate file-`static`s with no matching C struct (e.g. former `MathState` in `math.rs`, see the "MathState struct DELETED" comment there) are explicit anti-pattern below. |
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
| math eval | `Src/math.c` (file-statics) | `math.rs` `M_*` thread_local set |
| input stack | `Src/input.c` | `input.rs` thread_local (`SHIN`, …) |
| heap arena | `Src/mem.c` | `mem.rs` thread_local |
| exec context | `Src/context.c` | `context.rs` per-thread leaked `Mutex` slot (thread_local) |
| history build cursor | `Src/hist.c chline`/`chwords` | `hist.rs` `chline`/`chwords` — shared static `Mutex`, not thread_local |
| ZLE keymap state | `Src/Zle/zle_keymap.c` | `zle/zle_keymap.rs` file-scope `Mutex`/`OnceLock` statics (`curkeymap`, `KEYMAPNAMTAB`), not thread_local |
| fusevm bridge | (extension) | `fusevm_bridge.rs` thread_local |
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
| `paramtab` | `params.rs` `PARAMTAB_INNER` (accessor `paramtab()`) | `OnceLock<RwLock<hashtable_nodes<Param>>>` | done (Phase 3) |
| `jobtab` | `jobs.rs` `JOBTAB` | `OnceLock<Mutex<Vec<job>>>`; helpers such as `update_job`/`freejob` still take `&mut [job]` | holder done (Phase 3); call-site migration deferred |
| `cmdnamtab`, `shfunctab`, `aliastab`, `sufaliastab`, `reswdtab`, `histtab` | `hashtable.rs` `cmdnamtab_lock()`/`shfunctab_lock()`/`aliastab_lock()`/`sufaliastab_lock()`/`reswdtab_lock()`/`histtab_lock()` | `OnceLock<RwLock<…>>` each | done (Phase 3) |
| `hist_ring` (histlist) | `hist.rs` `hist_ring` | `Mutex<Vec<histent>>`; `chline`/`chwords` also `Mutex` | done (Phase 3) |
| `opts[]` | `options.rs` `OPTS_LIVE` | `OnceLock<RwLock<opt_state_store>>` (`flat: [i8; OPT_SIZE]` + `extra` map) | done (Phase 3) |
| `sigtrapped`, `siglists` | `signals.rs` `sigtrapped`/`siglists` | `LazyLock<Mutex<Vec<…>>>` | shared static; correct for bucket 2 |
| builtin table | `builtin.rs` `builtintab` | `OnceLock<HashMap<String, &'static builtin>>` | correct |

---

## Historical — Bucket 1 State Stored as `Mutex` (resolved in Phase 1)

These sites used `static Mutex<…>` for state that is logically
per-evaluator (bucket 1). The Mutex was correct under single-threaded
execution but (a) needlessly serialized worker threads doing parallel
parse/completion work, and (b) drifted from the C semantics — these
were file-`static`s in C, not lock-protected shared state. All are
resolved; current holders below.

| Symbol | Bucket-1 reason | Resolution |
|---|---|---|
| `prompt.rs` `CMDSTACK` | per-evaluator parser context (case/if/for nesting) | `thread_local!` |
| `pattern.rs` `PATTERN_SCOPES` | per-pattern-compile scope stack | replaced by `thread_local!` `PATSCOPE_STACK` |
| `zle/compctl.rs` `CMATCHER` | (reclassified) | bucket 2, `RwLock` |
| `zle/compctl.rs` `COMPCTL_TAB` | (reclassified) | bucket 2, `RwLock` |
| `zle/compctl.rs` `PATCOMPS` | (reclassified) | bucket 2, `RwLock` |
| `zle/compctl.rs` `CCLIST` | per-completion result list mode | `thread_local!` |
| `zle/compctl.rs` `SHOWMASK` | per-completion show mask | `thread_local!` |
| `INCOMPFUNC` | per-completion in-function flag | unified `utils.rs` `INCOMPFUNC` `AtomicI32` |
| `zle/compctl.rs` `INCOMPCTLFUNC` | per-completion in-compctl flag | `thread_local!` |
| `zle/compctl.rs` `ADDWHAT` | per-completion add-what flag | `thread_local!` |
| `zle/compctl.rs` `MATCH_LIST` | per-completion match buffer | `thread_local!` |
| `zle/compctl.rs` `PRPRE` | per-completion prefix-prefix | `thread_local!` |
| `zle/compctl.rs` `CDEPTH` | per-completion recursion depth | `thread_local!` |
| `zle/compctl.rs` `CCONT` | per-completion continuation flag | `thread_local!` |
| `zle/compctl.rs` `CMDSTR` | per-completion command string | `thread_local!` |
| `zle/compctl.rs` `CCUSED` | per-completion used set | `thread_local!` |
| `WE` | per-completion word-end | deduped to `zle/compcore.rs` `WE` `AtomicI32` |
| `WB` | per-completion word-begin | deduped to `zle/compcore.rs` `WB` `AtomicI32` |
| `ZLEMETACS` | per-completion meta cursor | deduped to `zle/compcore.rs` `ZLEMETACS` `AtomicI32` |

19 sites: 16 converted in Phase 1, 3 reclassified to bucket 2.
`WE`/`WB`/`ZLEMETACS` were later deduped (`530e9e481f`) into the
`compcore.rs` `AtomicI32` statics (C lex.c globals), and `INCOMPFUNC`
was unified in `93d0d33b23`.

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

### Canonical example (`math.rs::save_state`)

`math.rs` uses the `// !!! WARNING: RUST-ONLY HELPER !!!` header variant
of this marker; both header forms occur in `src/ported/` (count with
`rg -c 'WARNING: RUST-ONLY HELPER' src/ported` and
`rg -c '// WARNING: NOT IN [A-Z_]+\.C' src/ported`).

```rust
// !!! WARNING: RUST-ONLY HELPER !!! — no counterpart in Src/math.c.
// Rust-only helper. C inlines the
// xyy* save/restore directly inside `mathevall()`'s body
// (math.c:367 onward); the Rust port factors it out because two
// callsites (callmathfunc arg parsing, getmathparam indirect-string
// eval) would each duplicate ~17 lines of save/restore code.
fn save_state() -> xyy_locals {
    ...
}
```

### Relationship to PORT.md's existing WARNING marker

PORT.md prescribes `/// WARNING: THIS IS ADHOC IMPLEMENTATION AND
NOT A FAITHFUL PORT` for fns the adhoc detector flags as having no
C counterpart at all. That marker means *"this whole fn is unported
placeholder code, delete or port me."* No `src/` fn carries it as of
2026-10-27 (v0.13.56): `rg -n 'THIS IS ADHOC' src/` matches only the
prohibition in `src/ported/mod.rs`, and `modules/termcap.rs` carries
only `// WARNING: NOT IN TERMCAP.C` markers.

The shorter `// WARNING: NOT IN <FILE>.C` marker means
*"deliberate, justified Rust-only helper, do not delete."* Use the
short form when the helper is intentional and necessary; use the
long form when the fn is a stub waiting for a real port. They are
**not** interchangeable.

### Verification command

```sh
# List all ported under src/ported/ whose name is NOT in C:
rg -n '^\s*(pub\s+)?(pub\(crate\)\s+)?fn\s+(\w+)' src/ported/ \
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
`struct foo` to point at. The `math.rs` "MathState struct
DELETED" comment is the reference for how to undo this anti-pattern when
it appears.

### 2. `Mutex` for per-evaluator state

Using `Mutex` because "it works" under single-threaded test runs.
Once worker threads run in parallel, every parser/completion
invocation contends on the same lock. The 19 historical sites above
were this anti-pattern (all resolved).

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
per-evaluator in C. Each line names the Rust holder plus the C source
citation. Each converted holder
**must keep** its `/// Port of file-static <c_name> from
Src/<file>.c:<NNNN>` doc-comment per PORT.md Rule 2 (the citation
form already in place — see e.g. `zle/compctl.rs` `CMATCHER`).

PORT.md compliance: no new files, no new `fn` names. Only the
*holder primitive* changes (`Mutex` → `thread_local!`). Keep the
SCREAMING_SNAKE name verbatim from the C identifier.

**Verified bucket-1 (file-`static` in C, per-completion-call /
per-evaluator):**

- [x] `prompt.rs` `CMDSTACK` ← `Src/prompt.c:56` `cmdstack` (parser
      context stack) — `thread_local! RefCell<Vec<u8>>`
- [x] `pattern.rs` `PATSCOPE_STACK` ← `Src/pattern.c:274`
      `zpc_disables_stack` per-evaluator function-scope disable
      save-stack — `thread_local! RefCell<Vec<u32>>`
- [x] `zle/compctl.rs` `CCLIST` ← `Src/Zle/compctl.c:63`
      `static int cclist` — `thread_local! Cell<i32>`
- [x] `zle/compctl.rs` `SHOWMASK` ← `Src/Zle/compctl.c:66`
      `static unsigned long showmask` — `thread_local! Cell<u64>`
- [x] `INCOMPFUNC` ← `Src/utils.c:46`
      `mod_export int incompfunc` — converted to `thread_local!
      Cell<i32>` in Phase 1; since unified as the single `utils.rs`
      `INCOMPFUNC` `AtomicI32` (`93d0d33b23`), `complete.rs`
      re-exports it
- [x] `zle/compctl.rs` `INCOMPCTLFUNC` ← `Src/Zle/zle_main.c:54`
      `mod_export int incompctlfunc` — `thread_local! Cell<bool>`
      (converted from `AtomicBool`)
- [x] `zle/compctl.rs` `ADDWHAT` ← `Src/Zle/compctl.c:1749`
      `static int addwhat` — `thread_local! Cell<i32>`
- [x] `zle/compctl.rs` `MATCH_LIST` ← `Src/Zle/compctl.c`
      per-call match heap — `thread_local! RefCell<Vec<String>>`
- [x] `zle/compctl.rs` `PRPRE` ← `Src/Zle/compctl.c` per-call
      prefix-prefix — `thread_local! RefCell<Option<String>>`
- [x] `zle/compctl.rs` `CDEPTH` ← `Src/Zle/compctl.c` per-call
      depth — `thread_local! Cell<i32>`
- [x] `zle/compctl.rs` `CCONT` ← `Src/Zle/compctl.c` per-call
      continuation — `thread_local! Cell<u64>`
- [x] `zle/compctl.rs` `CMDSTR` ← `Src/Zle/compctl.c` per-call
      command string — `thread_local! RefCell<Option<String>>`
- [x] `zle/compctl.rs` `CCUSED` ← `Src/Zle/compctl.c` per-call
      used set — `thread_local! RefCell<Vec<Arc<Compctl>>>`
- [x] `WE` ← per-call word-end — converted to `thread_local! Cell<i32>`
      in Phase 1; since deduped (`530e9e481f`) into `zle/compcore.rs`
      `WE` `AtomicI32` (C lex.c global)
- [x] `WB` ← per-call word-begin — same; now `zle/compcore.rs` `WB`
      `AtomicI32`
- [x] `ZLEMETACS` ← per-call meta cursor — same; now
      `zle/compcore.rs` `ZLEMETACS` `AtomicI32`

**Reclassified to bucket 2 (do NOT move to TLS — they are user-
registered shared registries, not per-call scratch). Verified
against `Src/Zle/compctl.c`:**

- `zle/compctl.rs` `CMATCHER` — `Src/Zle/compctl.c:36
  static Cmlist cmatcher`. *File-static, but written by `compctl
  -M` (line 326-327: `freecmlist(cmatcher); cmatcher = cpcmlist(l);`)
  and read by every completion call.* User-registered matcher
  registry → bucket 2. Now `RwLock` (Phase 3).
- `zle/compctl.rs` `COMPCTL_TAB` — `Src/Zle/compctl.c:46
  HashTable compctltab`. *No `static` — extern global declared in
  zle.h, populated by `compctl name args`, read by every
  completion call.* User registry → bucket 2. Now `RwLock` (Phase 3).
- `zle/compctl.rs` `PATCOMPS` — `Src/Zle/compctl.c:51
  Patcomp patcomps`. *No `static` — extern global, list of
  pattern compctls registered via `compctl -p`.* User registry →
  bucket 2. Now `RwLock` (Phase 3).

These three moved to Phase 3 (bucket-2 holders) and are `RwLock`
statics in `zle/compctl.rs` (see the Phase 3 checklist).

**End of Phase 1:**

- [x] `python3 scripts/gen_port_report.py` — refreshed
      `docs/port_report.html` (2026-05-13: 4,757 rows, 2,392 ported —
      historical)
- [x] `python3 scripts/match_or_warn_modules.py` — zero new
      WARNINGs from Phase 1 edits (pre-existing uncommitted WARNINGs
      in unrelated files are not Phase 1 work)
- [x] `cargo build --lib` green
- [x] `cargo test --lib -- compctl pattern` green (96 passed, 0 failed —
      2026-05-12)

**Outstanding Phase 1 follow-up (not blocking phase close):**

- [x] `INCOMPFUNC` fragmentation — **RESOLVED (`93d0d33b23`).** The
  separate decls modelling the one C `mod_export int incompfunc`
  (`Src/utils.c:46`) are unified: the single holder is `utils.rs`
  `INCOMPFUNC` (`AtomicI32`), `complete.rs` re-exports it
  (`pub use crate::ported::utils::INCOMPFUNC`), and `compctl.rs` imports
  it from `complete`.

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

- [x] `init.rs:36` `ShellState` ← `Src/init.c` has 0 structs;
      aggregates `argv0`, `argzero`, `posixzero`, `mypid`, `ppid`,
      `shtty`, `lineno`, `path`, `fpath`, etc. — all individual
      C globals. Verdict: **bucket-1**, dissolve. **Status: DONE
      (verified 2026-05-12 absent via grep) — file-statics live
      in `init.rs` (`noexitct` … `use_exit_printed`).**
- [x] `subst.rs:201` `SubstState` ← `Src/subst.c` has 0 structs.
      Verdict: **bucket-1**. **Status: DONE — struct absent
      (verified 2026-05-12).**
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
      ORIGLINE, LASTPREBR, LASTPOSTBR, AUTOQ, MENUCMP, COMPPREF,
      VALIDLIST, SHOWAGAIN, LASTAMBIG, BASHLISTFIRST, AMENU, LINCMD)
      already present in zle_tricky.rs; `COMPQUOTE`/`COMPCONTEXT` live
      in `complete.rs` (C `complete.c`), `compfunc` in `compcore.rs`.**
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

- [x] `glob.rs` `GlobData` ← `Src/glob.c:168 struct globdata`
      (28 fields). Verdict: **rename** + verify field parity.
      **Status: DONE — now `glob.rs` `globdata` (the interim
      `GlobData` name was dissolved in `ac53b7b696`), with the C
      file-static `curglobdata` mirrored as `CURGLOBDATA`. Partial,
      documented field parity: the doc-comment above `globdata`
      explains the collapses (`gd_matchsz`/`gd_matchbuf`/`gd_matchptr`
      → `matches: Vec<gmatch>`); the `gf_*` flags are fields, not a
      separate options struct (no `GlobOptions` type exists). Any
      `gd_*` field still missing is glob-completeness work, not a
      bucket-1/3 issue.**
- [x] `keymaps.rs:11` `ZleState` ← `Src/Zle/zle_main.c:432`
      `struct ztmout` and `:1927 struct findfunc` — neither
      matches ZleState's role (top-level ZLE state aggregator).
      Verdict: **bucket-1**, dissolve into per-static
      `thread_local!`s. **Status: DONE — struct dissolved.**
- [x] `zle/zle_keymap.rs` `BindState` ← `Src/Zle/zle_keymap.c`
      has 5 structs (`keymapname`, `keymap`, `key`, etc.). None
      match BindState (binding-context aggregator). Verdict:
      **bucket-1**, dissolve. **Status: DONE — fake struct + flags
      deleted (`d20c5a14bd`). The C-faithful `struct bindstate`
      (`Src/Zle/zle_keymap.c:95-104`) is ported as `zle_keymap.rs`
      `bindstate` and used by `scanbindlist` / `bindlistout`.**
- [x] `zle/zle_utils.rs:166` `UndoState` ← `Src/Zle/zle_utils.c`
      has 2 structs (`zle_region`, `zle_position`). Neither maps
      to UndoState — C has `static struct change *changes` /
      `static struct change *curchange` as file-statics for the
      undo ring. Verdict: **bucket-1**, dissolve.
      **Status: DONE — struct dissolved.**
- [ ] `zle/zle_refresh.rs` `RefreshState` ←
      `Src/Zle/zle_refresh.c:815` `struct rparams`. Field-by-
      field verification needed; if matches, **rename** to
      `RParams`. Otherwise **bucket-1**.
      **Status: NOT DONE — it was moved to
      `src/extensions/zle_refresh_state.rs` in `cc2b57d31d`, then
      moved back in `489ebe26f3`; `RefreshState` and its companions
      (TextAttr / VideoBuffer / HighlightManager, Rust-only
      abstractions over zsh's `zattr u64` + `nbuf[]`/`obuf[]` flat
      arrays + discrete winw/winh/vcs/vln/lpromptw/rpromptw statics)
      are in `src/ported/zle/zle_refresh.rs` again. The C-faithful
      `rparams` (port of c:815) is also in zle_refresh.rs. Unification
      to the C flat-array model is still open.**
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
- [x] `modules/parameter.rs:704` `JobState` ←
      `Src/Modules/parameter.c:2179 struct pardef`. **Status:
      DONE — struct absent (verified 2026-05-12 via grep).** The
      `JobState` enum in `src/exec_jobs.rs` is a separate type
      (Running/Stopped/Done) modeling job lifecycle and is not the
      bag-of-globals dissolved here.

**Per-struct dissolution work** is one commit each — bucket-1
verdicts above are blast-radius-ordered: smallest (single-module
modules/*) first, largest (init/subst/loop) last. Each
dissolution must keep the surrounding `fn`-name set unchanged
(PORT.md compliance — bucket-1 conversion changes storage, not
identity).

**End of Phase 2:**

- [x] `python3 scripts/gen_port_report.py` — refreshed
      `docs/port_report.html` (2026-05-13)
- [x] `python3 scripts/match_or_warn_modules.py` — zero new
      WARNINGs from Phase 2 verification (no code changes in
      Phase 2 — the work was already done; only the plan checkboxes
      were stale)
- [x] `cargo build --lib` green (carried from Phase 1 close)
- [x] Affected test modules green (no full-suite run)

### Phase 3 — Bucket-2 holders (as underlying ports land)

PORT.md compliance: holders live in the existing `src/ported/<x>.rs`
that maps to `Src/<x>.c`. No new files. The Rust holder name
matches the C identifier (`PARAMTAB` ← `paramtab`, etc.). Per
PORT.md "Mirror globals as static mut / Mutex<...> / thread-locals
as needed for parity, not Rust elegance" — `Arc<RwLock>` is the
parity-preserving choice when threading forces shared mutation.

- [x] `params.rs` `PARAMTAB_INNER` → `OnceLock<RwLock<
      hashtable_nodes<Param>>>` ← `Src/params.c paramtab`. Accessor
      `paramtab()` returns `&'static RwLock<...>`; ~80 call sites
      across 14 files routed through `.read()`/`.write()`. Done
      2026-05-12.
- [x] `hashtable.rs` `cmdnamtab_lock()` → function-local `static
      CMDNAMTAB: OnceLock<RwLock<cmdnam_table>>` ← `Src/hashtable.c:594
      cmdnamtab`. Call sites use `.read()` for lookups, `.write()` for
      `hash_dir`/`remove`/`clear`. Done 2026-05-12.
- [x] `hashtable.rs` `shfunctab_lock()` → `OnceLock<RwLock<
      shfunc_table>>` ← `Src/hashtable.c:808 shfunctab`. Done.
- [x] `hashtable.rs` `aliastab_lock()` → `OnceLock<RwLock<alias_table>>`
      ← `Src/hashtable.c:1186 aliastab`. Done.
- [x] `hashtable.rs` `sufaliastab_lock()` → `OnceLock<RwLock<
      alias_table>>` ← `Src/hashtable.c:1187 sufaliastab`. Done.
- [x] `hashtable.rs` `reswdtab_lock()` → `OnceLock<RwLock<reswd_table>>`
      ← `Src/hashtable.c:1115 reswdtab`. Done.
- [x] `hashtable.rs` `histtab_lock()` → `OnceLock<RwLock<
      HashMap<String,i32>>>` ← `Src/hashtable.c:1340 histtab`. Done.
- [x] `hist.rs` `hist_ring` → `Mutex<Vec<histent>>` ← `Src/hist.c
      :103 mod_export Histent hist_ring`. **Status: already
      bucket-2 compliant** (single shared `pub static Mutex<...>`,
      semantically equivalent to `Arc<Mutex>` for static storage —
      Arc adds value only for runtime-shared ownership). Done.
- [x] `options.rs` `OPTS_LIVE` → `OnceLock<RwLock<opt_state_store>>`
      ← `Src/options.c:36 opts[]`. Accessors
      `opt_state_get`/`opt_state_set` updated to `.read()`/
      `.write()`. Done 2026-05-12.
- [x] `jobs.rs` `JOBTAB` → `OnceLock<Mutex<Vec<job>>>` ← `Src/
      jobs.c:88 jobtab`. **Status: already bucket-2 compliant**
      (`Mutex` correct for high-mutation table per PORT_PLAN.md;
      `pub static OnceLock<Mutex<>>` is single shared instance).
      Call-site migration from `&mut [Job]` params is deferred per
      original plan note ("promote when daemon owns jobs reaping").
- [x] `zle/compctl.rs` `CMATCHER` → `RwLock<Option<Arc<Cmlist>>>`
      ← `Src/Zle/compctl.c:36 static Cmlist cmatcher`. Done. `Arc`, not
      `Box`: `Cmlist` is a POINTER type in C (`Src/Zle/comp.h:147`) and the
      `mstack` / `bmatchers` readers copy the pointer per candidate and per
      character, so an owning `Box` turned each of those reads into a
      recursive deep copy of the whole matcher chain.
- [x] `zle/compctl.rs` `COMPCTL_TAB` → `RwLock<Option<HashMap<
      String,Arc<Compctl>>>>` ← `Src/Zle/compctl.c:46 HashTable
      compctltab`. Done.
- [x] `zle/compctl.rs` `PATCOMPS` → `RwLock<Vec<(String,Arc<
      Compctl>)>>` ← `Src/Zle/compctl.c:51 Patcomp patcomps`. Done.

**End of Phase 3:**

- [x] `python3 scripts/gen_port_report.py` — refreshed
      `docs/port_report.html` (2026-05-13: 4,757 rows, 2,392 ported)
- [x] `python3 scripts/match_or_warn_modules.py` — zero new
      WARNINGs from Phase 3 storage-primitive promotions
- [x] `cargo build --lib` green
- [x] `cargo test --lib -- params hashtable options jobs compctl`
      green (156 passed, 0 failed — 2026-05-12)
- [x] `tests/shared_state_visible.rs` exists and passes — landed in
      Phase 5 (4/4 passing 2026-05-12)

**Pre-existing test failures noted at Phase 3 close (2026-05-12, HEAD
`9eedf60427`, a hash that no longer resolves; not Phase 3 regressions — neither test touches the
promoted holders):**
- `ported::builtin::tests::registration_table_matches_c_count` — then
  hardcoded `BUILTINS.len() == 82` against an actual 112. The
  assertion is now a hardcoded 160 (159 ported builtins + the
  zshrs-native `__rust_compile`), kept as a tripwire that is bumped
  alongside each change to the eagerly-loaded-module list. Pass/fail
  status not re-run for this update.
- `ported::lex::tests::test_function_tokens` — `{` lexed to token
  34 (STRING) instead of expected 41 (INBRACE_TOK). Status not
  re-run for this update.

### Phase 4 — Daemon image holders

Design: `docs/DAEMON.md`. Daemon code lives in the `daemon/`
workspace crate (`zshrs-daemon`; zsh has no daemon — this is a
sanctioned PORT.md Rule 1 exception #1, "features that zsh C does
not have"). The daemon ships (default `daemon` feature), but none of
the holders below are implemented: no ported holder in
`src/ported/<x>.rs` reads a daemon-owned mmap yet. The plan is for
each to read through a thin bridge outside `src/ported/`.

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

- [x] `tests/shared_state_visible.rs` — N=8 worker threads, each
      mutates a unique key in paramtab + opts; observer reads back
      all N mutations. Includes cross-thread-write-then-read pin and
      stress test (8 writers × 8 readers × 64 iters). Fails if a
      bucket-2 holder gets demoted to TLS. **Verifies:** zsh C
      single-process semantics for `Src/params.c paramtab` and
      `Src/options.c opts[]` preserved across worker threads.
      4/4 passing 2026-05-12.
- [x] `tests/per_evaluator_isolation.rs` — N=8 worker threads each
      run distinct `$((expr))` expressions through `mathevali`;
      verifies no cross-thread leak via the math TLS set. Includes
      nested-recursion stress + same-complex-expression-deterministic
      pin. **Verifies:** `Src/math.c` file-statics (`unary, noeval,
      lastbase, yyval, yylval, operand stack`) behave per-evaluator
      under threading, as they do per-process in C. 3/3 passing
      2026-05-12.
- [x] Both test files cross-referenced from
      `tests/tree_walker_absent.rs` doc-comment (per CLAUDE.md
      "96-test invariant is load-bearing"). Cargo's `tests/*.rs`
      auto-discovery wires them into the suite — no manual list
      maintenance needed.

**End of Phase 5:**

- [x] `cargo test --test shared_state_visible --test
      per_evaluator_isolation` green (7 total: 4 + 3 — 2026-05-12)
- [x] `python3 scripts/gen_port_report.py` — refreshed

### Phase 6 — `ShellExecutor` field dissolution (in flight)

`pub struct ShellExecutor` (now in `src/vm_helper.rs`; it was in
`src/exec.rs`, which no longer exists) had 61 fields at extraction
(`a8353f17d4`) and has 47 as of 2026-10-27 (count command in the
Progress Summary). Many duplicate canonical zsh C globals already
ported elsewhere.
Each duplicate is a Rule 1 violation: there is no `struct executor`
in zsh's C source — the equivalent state lives as file-statics across
`Src/exec.c`, `Src/init.c`, `Src/options.c`, `Src/hashtable.c`,
`Src/prompt.c`, etc. The campaign deletes one field per commit,
migrates every caller to the canonical zsh global, and removes the
duplicate.

**Recorded deletions (most-recent first, 16 fields in 12 commits):**

Hashes are the commits on `main`; the hashes previously recorded here
no longer resolve.

| Commit | Field | Routed to |
|---|---|---|
| `9b82895611` | `zwc_cache` | field and its init removed (in a comment-cleanup commit) |
| `59a701ee8d` | `loop_signal` (+ Rust-only `LoopSignal` enum) | canonical `BREAKS` / `CONTFLAG` atomics in `builtin.rs` (`Src/loop.c:41,46`) |
| `c49ef1a907` | `autoload_pending` | canonical `shfunctab` + `PM_UNDEFINED` bit (`Src/exec.c:5215`) |
| `75cb96a604` | `options` (60+ callers) | `OPTS_LIVE` via `opt_state_get/_set/_unset/_snapshot/_len` |
| `4abfbfc3c7` | `cmd_stack` | `prompt::CMDSTACK` TLS via `cmdpush/cmdpop` (`Src/prompt.c:1620,1631`) |
| `fa8c8b6488` | `hook_functions` | `<hook>_functions` paramtab arrays (zsh `add-zsh-hook` idiom) |
| `1865424bec` | `command_hash` (never-populated dup) | `cmdnamtab_lock` |
| `6caa8e7b57` | `named_dirs` | `nameddirtab` Mutex in `src/ported/hashnameddir.rs` |
| `baa5fa75b4` | `readonly_vars` (never-populated dup) | `PM_READONLY` flag bit |
| `1082164240` | `dir_stack` | `DIRSTACK` Mutex in `modules/parameter.rs` (`Src/builtin.c:1456`) |
| `fdbf47e3e9` | `last_subst` / `sub_flags` / `in_paramsubst_nest` / `zftp` / `style_table` (5 fields) | `IN_PARAMSUBST_NEST` TLS + canonical statics |
| `0cfaa1a086` | `expanding_aliases` (fake HashSet) | `alias.inuse` bump/clear at `fusevm_bridge.rs` call sites |

This table is not the full history. Diffing the field names at
`a8353f17d4:src/exec.rs` against `src/vm_helper.rs` (2026-10-27): 43
of the 61 original fields are gone (10 of them are in the table; the
other 33, e.g. `variables`, `functions`, `traps`, `comp_*`, went in
unrecorded commits), and 29 of today's fields were added after
extraction (e.g. `scriptname`, `redirect_scope_stack`, the `ztest_*`
counters). The other 6 table rows (`loop_signal`, `cmd_stack`,
`last_subst`, `sub_flags`, `in_paramsubst_nest`, `expanding_aliases`)
were also added after extraction, then deleted. 61 − 43 + 29 = 47.

**Remaining fields (47 as of 2026-10-27):** list them with
`awk '/^pub struct ShellExecutor/,/^}/' src/vm_helper.rs | grep -oE
'^\s*(pub )?[a-z_]+:[^:]'`. They include `scriptname`,
`scriptfilename`, `subshell_snapshots`, `inline_env_stack`,
`current_command_glob_failed`, `jobs`, `fpath`, `history`, the
redirect-scope stacks, the function-source tables, the `ztest_*`
counters, and zshrs-only state (`worker_pool`, `intercepts`,
`async_jobs`, `plugin_cache`, `compsys_cache`). Each requires audit:
(a) does the canonical zsh global exist? (b) if yes, migrate callers
+ delete. (c) if no, keep + cite the C file-static it represents.

**Per-deletion discipline:**
1. Identify the canonical zsh C global (file-static or hash table).
2. Verify the canonical Rust holder exists (`paramtab`, `cmdnamtab`,
   `OPTS_LIVE`, `prompt::CMDSTACK`, `aliastab`, etc.).
3. Migrate every caller of `executor.<field>` to the canonical
   accessor.
4. Delete the field from `ShellExecutor`.
5. Commit citing the C source line.

**End of Phase 6:** the `pub struct ShellExecutor` shrinks to fields
that genuinely have no canonical zsh global (or are zshrs-specific
extensions explicitly outside zsh's design — e.g. `worker_pool`,
`intercepts`).
Document each surviving field's justification inline.

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
sneaks back in for a bucket-2 holder, this test fails. That test
is `tests/shared_state_visible.rs` (paramtab + opts across worker
threads); its per-evaluator counterpart is
`tests/per_evaluator_isolation.rs`. Both landed in Phase 5.
