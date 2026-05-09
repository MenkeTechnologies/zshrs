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
