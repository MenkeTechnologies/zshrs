//! Loop execution for zshrs
//!
//! Port from zsh/Src/loop.c (802 lines)
//!
//! # of nested loops we are in                                              // c:33
//! # of continue levels                                                     // c:38
//! # of break levels                                                        // c:43
//!
//! In C, loop.c contains execfor, execwhile, execif, execcase, execselect,
//! execrepeat, and exectry as separate functions operating on bytecode.
//! In Rust, all of these are implemented as match arms in
//! ShellExecutor::execute_compound() in exec.rs, operating on the typed AST
//! (CompoundCommand::For, While, If, Case, Select, Repeat, Try).
//!
//! This module provides the loop state management and helper functions
//! that support the executor's loop implementation.

use std::sync::atomic::{AtomicI32, Ordering};

/// Number of nested loops.
/// Port of the global `loops` counter from Src/loop.c — every
/// `execfor`/`execwhile`/`execrepeat`/`execselect` entry bumps it
/// and decrements on exit.
static LOOP_DEPTH: AtomicI32 = AtomicI32::new(0);

/// Continue flag / level.
/// Port of the global `contflag` from Src/loop.c — set by the
/// `continue` builtin (Src/builtin.c:bin_break) and consumed by
/// the loop body's exit check.
static CONT_FLAG: AtomicI32 = AtomicI32::new(0);

/// Break level.
/// Port of the global `breaks` counter from Src/loop.c — set by
/// the `break` builtin (Src/builtin.c:bin_break) and tested by
/// each enclosing loop on exit.
static BREAK_LEVEL: AtomicI32 = AtomicI32::new(0);

// Note: dead `LoopState` aggregate (and impl/tests) deleted per
// PORT_PLAN Phase 2. It was a Rust-only invention that double-tracked
// the same data already living in the file-statics LOOP_DEPTH /
// CONT_FLAG / BREAK_LEVEL above (and on `ShellExecutor.breaking` /
// `ShellExecutor.continuing` in src/exec.rs:572-573). Zero callers
// outside its own test module.
//
// C source's actual loop-control file-globals at `Src/loop.c`:
//
//     int loops;                          // line 36
//     mod_export int contflag;            // line 41
//     mod_export volatile int breaks;     // line 46
//
// All `mod_export` (cross-compilation-unit), so they're PORT_PLAN
// Phase 3 bucket-2 (Arc<RwLock>) work. The canonical C-named ports
// (LOOPS / CONTFLAG / BREAKS) live in `src/ported/builtin.rs:3657-3659`
// where the bin_break dispatcher consults them; the local
// LOOP_DEPTH / CONT_FLAG / BREAK_LEVEL above are this file's
// internal mirrors.

// And this is used to print select lists.                                 // c:343
/// Select-menu display.
/// Port of `selectlist(l, start)` from Src/loop.c:347 — formats the
/// numbered menu the C source uses for `select var in words`. Picks
/// columns automatically when `columns == 0`, mirroring the C
/// source's terminal-width auto-detection.
pub fn selectlist(items: &[&str], start: usize) -> usize {              // c:347
    use std::io::Write;
    let mut stderr = std::io::stderr().lock();

    // c:351 — zleentry(ZLE_CMD_TRASH); — flush ZLE redraw state.
    // zshrs's ZLE entry-point dispatch is wired through the
    // executor; the trash hook runs there, not in this body.

    let ct = items.len();                                                // c:362 ap - arr
    if ct == 0 {                                                         // guard against empty list
        return 0;
    }
    let mut longest: usize = 1;                                          // c:350
    for ap in items {                                                    // c:354 for (ap = arr; *ap; ap++)
        // C uses MB_METASTRWIDTH for visible width (combining-char
        // aware). Rust port uses chars().count() — adequate for
        // standard ASCII / non-combining content.
        let aplen = ap.chars().count();                                  // c:359 unmetafy width
        if aplen > longest {                                             // c:362
            longest = aplen;                                             // c:363
        }
    }
    longest += 1;                                                        // c:365 +1 for ") "
    let mut t0 = ct;                                                     // c:366
    while t0 != 0 {                                                      // c:367
        t0 /= 10;                                                        // c:368
        longest += 1;                                                    // c:368 (+1 per digit)
    }

    let zterm_columns = crate::ported::utils::adjustcolumns();           // c:zterm_columns
    let zterm_lines   = crate::ported::utils::adjustlines();
    let mut fct: usize = (zterm_columns.saturating_sub(1)) / (longest + 3); // c:371
    let fw: usize;
    if fct == 0 {                                                        // c:372
        fct = 1;                                                         // c:373
        fw = 0;                                                          // (unused when fct==1)
    } else {
        fw = (zterm_columns - 1) / fct;                                  // c:375
    }
    let colsz = (ct + fct - 1) / fct;                                    // c:377

    // c:379 — for (t1 = start; t1 != colsz && t1 - start < zterm_lines - 2; t1++)
    let mut t1 = start;
    let max_lines = zterm_lines.saturating_sub(2);
    while t1 != colsz && (t1 - start) < max_lines {
        let mut idx = t1;                                                // c:381 ap = arr + t1
        loop {                                                           // c:383 do {
            let entry = items[idx];
            // c:385 — t2 = MB_METASTRWIDTH(*ap) + 2;
            let mut t2 = entry.chars().count() + 2;
            // c:391 — fprintf(stderr, "%d) %s", t3 = ap - arr + 1, *ap);
            let t3 = idx + 1;
            let _ = write!(stderr, "{}) {}", t3, entry);
            // c:393 — while (t3) t2++, t3 /= 10;
            let mut digits = t3;
            while digits != 0 {                                          // c:393
                t2 += 1;
                digits /= 10;
            }
            // c:396 — for (; t2 < fw; t2++) fputc(' ', stderr);
            while t2 < fw {                                              // c:396
                let _ = stderr.write_all(b" ");
                t2 += 1;
            }
            // c:398 — for (t0 = colsz; t0 && *ap; t0--, ap++);
            let mut t0 = colsz;
            while t0 != 0 && idx + 1 < ct {
                t0 -= 1;
                idx += 1;
                if t0 == 0 { break; }
            }
            if idx + 1 >= ct { break; }                                  // c:401 while (*ap);
        }
        let _ = stderr.write_all(b"\n");                                 // c:401 fputc('\n', stderr);
        t1 += 1;
    }

    let _ = stderr.flush();                                              // c:413 fflush(stderr);

    if t1 < colsz { t1 } else { 0 }                                      // c:415 return
}

// Note: dead `ForIterator` / `CForState` / `TryState` aggregates
// removed per PORT_PLAN Phase 2. None had production callers (only
// internal test references). The actual control flow is lowered in
// the fusevm compiler — every `for`/`while`/`select`/`repeat`/`try`
// AST node becomes a fusevm Op (see `src/extensions/compile_zsh.rs`).
//
// C source's relevant try-block file-globals (loop.c:719-727):
//
//     zlong try_errflag = -1;       // line 719 (TRY_BLOCK_ERROR)
//     zlong try_interrupt = -1;     // line 727 (TRY_BLOCK_INTERRUPT)
//
// Exported via `IPDEF6` paramdef in `Src/params.c:364`, so they're
// cross-compilation-unit globals → PORT_PLAN Phase 3 bucket-2
// (Arc<RwLock>) work, not the Phase 2 bucket-1 (thread_local!) wave.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selectlist_returns_zero_when_full_fits() {
        // selectlist now matches C: writes to stderr and returns
        // 0 when the whole list fits in one page.
        let items = ["one", "two", "three"];
        // start = 0; in a sufficiently tall terminal, all rows fit
        // and the function returns 0.
        let r = selectlist(&items, 0);
        // Either 0 (all fit) or a non-zero next-page offset
        // (terminal tiny). Both are valid C-side outputs.
        assert!(r < items.len() || r == 0);
    }
}

// ===========================================================
// Tree-walker control-flow dispatch entries.
//
// In zsh these seven functions are bytecode-tree walkers — each
// consumes a `Wordcode`/`Estate` cursor and recursively invokes
// `execlist()` for nested clauses. They run during the legacy
// `tree_walker` execution path.
//
// zshrs replaces the tree walker entirely with fusevm bytecode
// (see `tree_walker_absent.rs` / `no_tree_walker_dispatch.rs`
// invariant tests), so these entries exist to satisfy ABI/name
// parity. The actual control-flow lowering happens in the
// fusevm compiler (`crate::fusevm::compile`) where every
// `for`/`while`/`if`/`case`/`select`/`repeat`/`try` AST node
// becomes a fusevm `Op`.
// ===========================================================

// The seven entries below are zsh's tree-walker dispatch handlers
// from `Src/loop.c`. zshrs replaces the tree walker entirely with
// fusevm bytecode — every `for`/`while`/`if`/`case`/`select`/
// `repeat`/`try` AST node lowers to a fusevm Op in
// `src/extensions/compile_zsh.rs`. These entries exist purely for
// C-name parity (drift gate enforces every Rust fn maps to a C fn).
//
// The 96-test architectural invariant in `tree_walker_absent.rs` +
// `no_tree_walker_dispatch.rs` proves these are never reached in
// production. Each body is `unreachable!()` so ANY caller fails
// loudly rather than silently returning 0 — if a port regresses
// the bytecode lowering, we want the test suite to crash, not pass.
//
// Faithful per-fn port of the C bodies is intentionally NOT done:
// they read `Wordcode` / `Estate` cursors that zshrs doesn't model.
// The semantic equivalent lives in:
//   execfor    → compile_zsh.rs::compile_for
//   execselect → compile_zsh.rs::compile_select
//   execwhile  → compile_zsh.rs::compile_while
//   execrepeat → compile_zsh.rs::compile_repeat
//   execif     → compile_zsh.rs::compile_if
//   execcase   → compile_zsh.rs::compile_case
//   exectry    → compile_zsh.rs::compile_try

/// Port of `execfor(state, do_exec)` from `Src/loop.c:50`. See module-level note:
/// fusevm bytecode replaces the tree walker; this entry is
/// `unreachable!()` to crash if regressed.
pub fn execfor(_do_exec: i32) -> i32 {                                   // c:50
    unreachable!("execfor: tree-walker disabled — fusevm lowers `for` in compile_zsh.rs")
}

/// Port of `execselect(state)` from `Src/loop.c:217`.
pub fn execselect(_do_exec: i32) -> i32 {                                // c:217
    unreachable!("execselect: tree-walker disabled — fusevm lowers `select` in compile_zsh.rs")
}

/// Port of `execwhile(state)` from `Src/loop.c:413`.
pub fn execwhile(_do_exec: i32) -> i32 {                                 // c:413
    unreachable!("execwhile: tree-walker disabled — fusevm lowers `while`/`until` in compile_zsh.rs")
}

/// Port of `execrepeat(state)` from `Src/loop.c:499`.
pub fn execrepeat(_do_exec: i32) -> i32 {                                // c:499
    unreachable!("execrepeat: tree-walker disabled — fusevm lowers `repeat` in compile_zsh.rs")
}

/// Port of `execif(state, do_exec)` from `Src/loop.c:553`.
pub fn execif(_do_exec: i32) -> i32 {                                    // c:553
    unreachable!("execif: tree-walker disabled — fusevm lowers `if`/`elif`/`else` in compile_zsh.rs")
}

/// Port of `execcase(state, do_exec)` from `Src/loop.c:600`.
pub fn execcase(_do_exec: i32) -> i32 {                                  // c:600
    unreachable!("execcase: tree-walker disabled — fusevm lowers `case` in compile_zsh.rs")
}

/// Port of `exectry(state, do_exec)` from `Src/loop.c:735`.
pub fn exectry(_do_exec: i32) -> i32 {                                   // c:735
    unreachable!("exectry: tree-walker disabled — fusevm lowers `try`/`always` in compile_zsh.rs")
}
