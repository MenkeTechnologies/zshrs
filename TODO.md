# TODO — porting gaps that block 100% C-faithful ports

This file tracks port gaps where a function in `src/ported/` cannot
yet be faithfully ported because it depends on a C primitive that
hasn't been ported. Each entry blocks at least one downstream file
from being marked as 100% line-by-line in `docs/PORT_CHECKLIST.md`.

When an item is fixed: port the dependency, fix every blocked
function, then strike through (or delete) the entry here.

---

## Lexer-context machinery (`Src/lex.c`)

C functions that drive standalone lexer walks over a buffer —
`zcontext_save()`, `zcontext_restore()`, `ctxtlex()`, `inpush()`,
`strinbeg()`, `strinend()` — are not yet ported with their
side-effect-driven token-stream API. zshrs lowers the lexer through
fusevm bytecode and does not currently expose a "tokenise this
string" entry point.

**Blocks:**
- `zle/textobjects.rs::selectargument` — the `select-in-shell-word`
  argument-N selector at `Src/Zle/textobjects.c:212`. Body uses
  `ctxtlex()` over an inpush'd line buffer to find argument
  boundaries respecting quoting and expansion. Current Rust port
  is a whitespace-split approximation that handles only the simple
  no-quote case.

---

## `virangeflag` file-global (`Src/Zle/zle_vi.c:36`)

Cross-compilation-unit int set during vi-operator-pending
evaluation. Used by `selectword` (textobjects.c:196) and several
zle_move / zle_word fns to skip the trim-cursor adjustment when
the widget is being invoked as part of a vi range op.

**Blocks:**
- `zle/textobjects.rs::selectword` — the cursor-adjustment arm at
  `c:196-203` reads `virangeflag` to decide whether to set
  `region_active = 1` (emacs-mode default) or `DECCS()` (vi-cmd
  mode). Current Rust port treats it as constant-false.

**Fix path:** PORT_PLAN Phase 3 bucket-2 wave (Arc<RwLock<i32>>
or AtomicI32 file-static; needs paramdef wiring since it's also
exposed via the param table).

---

## Rust-only-types-still-present in src/ported/

These files have one or more `pub struct` / `pub enum` whose name
doesn't appear in the matching C source. They violate strict-rule 1
("zero Rust-only structs/enums") and need rewrite before being
ticked DONE in PORT_CHECKLIST.md:

- `modules/zselect.rs` — `SelectMode` enum, `ZselectOptions`
  struct, `SelectResult` struct. C zselect.c has zero structs/enums.
  Fix path: rewrite `bin_zselect` as a single fn that does inline
  flag parsing + select() call, matching `bin_zselect()` at
  zselect.c:67-200 line-by-line. ~300-line rewrite.

- `modules/langinfo.rs` — DONE. Earlier `LangInfoItem` already
  deleted; liitem already returns raw `Option<libc::nl_item>`.
  Tests pass: 5/5.

- `modules/ksh93.rs` — DONE. Earlier `Ksh93Params`/`NamerefOptions`
  already deleted; matchgetfn signature updated to take
  `&ShellExecutor` so the param-table reads happen through
  `exec.arrays`/`exec.variables` rather than `std::env::var` (zsh
  shell arrays aren't env vars).
- `modules/stat.rs` — DONE (commit `4ae9cff069`).
- `modules/mapfile.rs` — DONE (commit `a2d70f4776`).
- `modules/hlgroup.rs` — PARTIAL (commit `a4d3925a6a`); blocked
  on prompt.c match_highlight + zattrescape ports — see above.
- `modules/zprof.rs` — DONE (commit `92be6c235d`); Profiler bag-of-
  globals dissolved into module-level CALLS/NCALLS/ARCS/NARCS/
  STACK/ZPROF_MODULE statics matching C file-statics.

---

## prompt.c match_highlight + zattrescape — Rust-only signatures

`Src/prompt.c:2031 match_highlight()` and `Src/prompt.c:257
zattrescape()` are present in `src/ported/prompt.rs` but with
Rust-only signatures and Rust-only `TextAttrs` shape:

- `match_highlight(spec: &str) -> (TextAttrs, TextAttrs)` —
  C signature is `int match_highlight(const char *teststr,
  zattr *on_var, zattr *setmask, int *layer)` returning the
  number of bytes consumed and writing to out-args.
- `zattrescape(attrs: &TextAttrs) -> String` — C signature is
  `char *zattrescape(zattr atr, int *len)` returning a newly-
  allocated `\033[...m` escape stream (the `len` out-arg holds
  the byte length). Current Rust port returns `%`-prefixed
  prompt syntax, not ANSI escapes.

Both are downstream of an unported `zattr` integer type and the
`Color`/`TextAttrs` Rust-only abstractions in prompt.rs.

**Blocks:**
- `modules/hlgroup.rs::convertattr` — c:46-47 calls
  `match_highlight(attrstr, &atr, NULL, NULL)` then
  `zattrescape(atr, sgr ? NULL : &len)`. Rust port currently
  inlines the highlight + colour parsing because the prompt.rs
  ports don't speak the C protocol. Move to a real chain when
  the prompt.rs ports are made C-faithful.
- `modules/hlgroup.rs::getgroup` and `::scangroup` — both
  read the `$.zle.hlgroups` (`GROUPVAR`) hashtable through
  `getvalue()` + the magic-assoc dispatch. Pending the real
  Param/HashTable port, both bodies are the C "no entry"
  branch (None / empty Vec).

**Fix path:** rewrite prompt.rs's `match_highlight` and
`zattrescape` from `Src/prompt.c:257`/`:2031` line-by-line.
Replace `TextAttrs` with the C `zattr` integer bitmask. Then
hlgroup.rs::convertattr collapses to the 3-call chain in c:42-77.

---

## Module-loader signatures (need `&mut ShellExecutor`)

Every C module's `setup_()` / `features_()` / `enables_()` / `boot_()`
/ `cleanup_()` / `finish_()` takes a `Module m` arg and reads/writes
shell-wide state (param table, fdtable, function-wrapper list,
emulation mode). zshrs's free-fn signatures `pub fn boot_() -> i32`
have no access to that state.

Files where this gap means boot_/cleanup_/etc. is a partial port:
- `modules/newuser.rs::boot_` — needs `EMULATION(EMULATE_ZSH)` check
  + `source(buf)` for the newuser-install-script probe (newuser.c:67-103).
- `modules/ksh93.rs::cleanup_` — needs `deletewrapper(m, wrapper)` +
  paramtab walk to clear `PM_NAMEREF` flags (ksh93.c:265-281).
- `modules/example.rs::boot_` — needs `addwrapper(m, wrapper)`
  (example.c:222-228).
- (More to come as the audit progresses.)

**Fix path:** introduce a `&mut ShellExecutor` parameter on the module-
loader signatures, threading it through the dispatcher. This is
project-wide and should land in one commit.

---

## Note: this file is the only place to mention port gaps

If a function in `src/ported/` is not 100% line-by-line, it
must NOT be ticked in `docs/PORT_CHECKLIST.md`. Add the gap
here, link from the file's checklist entry, and keep moving.
