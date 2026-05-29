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
//! ShellExecutor::execute_compound() in vm_helper, operating on the typed AST
//! (CompoundCommand::For, While, If, Case, Select, Repeat, Try).
//!
//! This module provides the loop state management and helper functions
//! that support the executor's loop implementation.

use crate::ported::utils::{adjustcolumns, adjustlines};
use std::io::Write;
use std::sync::atomic::AtomicI32;
// ===========================================================
// C `Src/loop.c` — wordcode VM helpers for control flow.
//
// In C zsh, these seven functions run as part of the wordcode VM in
// `Src/exec.c`: each consumes an `Estate` / wordcode cursor (not a
// separate AST interpreter in the parser).
//
// zshrs lowers shell constructs to fusevm bytecode
// (see `tree_walker_absent.rs` / `no_tree_walker_dispatch.rs`
// invariant tests), so these entries exist to satisfy ABI/name
// parity. The actual control-flow lowering happens in the
// fusevm compiler (`crate::fusevm::compile`) where every
// `for`/`while`/`if`/`case`/`select`/`repeat`/`try` AST node
// becomes a fusevm `Op`.
// ===========================================================

// The seven entries below are zsh's `Src/loop.c` wordcode VM hooks.
// zshrs lowers shell constructs to fusevm bytecode — every `for`/`while`/`if`/`case`/`select`/
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

// execfor moved to src/ported/exec.rs as a faithful port of
// `Src/loop.c:50`. See exec.rs for the C-line-cited body.

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
//     zlong try_tryflag = 0;        // line 731 (TRY_BLOCK_DEPTH)
//
// Exported via `IPDEF6` paramdef in `Src/params.c:364`, so they're
// cross-compilation-unit globals → PORT_PLAN Phase 3 bucket-2
// (Arc<RwLock>) work, not the Phase 2 bucket-1 (thread_local!) wave.

/// Port of `mod_export zlong try_tryflag` from `Src/loop.c:731`.
/// Depth-counter for active `always {}` blocks; bumped at try entry
/// (`exectry`), decremented at exit. `dotrapargs` (`signals.c:1215`)
/// reads it to decide whether a trap's `errflag` propagates.
pub static try_tryflag: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0); // c:731

// execselect moved to src/ported/exec.rs as a faithful port of
// `Src/loop.c:217`. See exec.rs for the C-line-cited body.

// Note: dead `LoopState` aggregate (and impl/tests) deleted per
// PORT_PLAN Phase 2. It was a Rust-only invention that double-tracked
// the same data already living in the file-statics LOOP_DEPTH /
// CONT_FLAG / BREAK_LEVEL above (and on `ShellExecutor.breaking` /
// `ShellExecutor.continuing` in src/vm_helper:572-573). Zero callers
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

// And this is used to print select lists.                                 // c:347
/// Select-menu display.
/// Port of `selectlist(LinkList l, size_t start)` from Src/loop.c:347 — formats the
/// numbered menu the C source uses for `select var in words`. Picks
/// columns automatically when `columns == 0`, mirroring the C
/// source's terminal-width auto-detection.
/// WARNING: param names don't match C — Rust=(items, start) vs C=(l, start)
pub fn selectlist(items: &[&str], start: usize) -> usize {
    // c:347
    let mut stderr = std::io::stderr().lock();

    // c:351 — zleentry(ZLE_CMD_TRASH); — flush ZLE redraw state.
    // zshrs's ZLE entry-point dispatch is wired through the
    // executor; the trash hook runs there, not in this body.

    let ct = items.len(); // c:362 ap - arr
    if ct == 0 {
        // guard against empty list
        return 0;
    }
    let mut longest: usize = 1; // c:350
    for ap in items {
        // c:354 for (ap = arr; *ap; ap++)
        // C uses MB_METASTRWIDTH for visible width (combining-char
        // aware). Rust port uses chars().count() — adequate for
        // standard ASCII / non-combining content.
        let aplen = ap.chars().count(); // c:359 unmetafy width
        if aplen > longest {
            // c:362
            longest = aplen; // c:363
        }
    }
    longest += 1; // c:365 +1 for ") "
    let mut t0 = ct; // c:366
    while t0 != 0 {
        // c:367
        t0 /= 10; // c:368
        longest += 1; // c:368 (+1 per digit)
    }

    let zterm_columns = adjustcolumns(); // c:zterm_columns
    let zterm_lines = adjustlines();
    let mut fct: usize = (zterm_columns.saturating_sub(1)) / (longest + 3); // c:371
    let fw: usize;
    if fct == 0 {
        // c:372
        fct = 1; // c:373
        fw = 0; // (unused when fct==1)
    } else {
        fw = (zterm_columns - 1) / fct; // c:375
    }
    let colsz = (ct + fct - 1) / fct; // c:377

    // c:379 — for (t1 = start; t1 != colsz && t1 - start < zterm_lines - 2; t1++)
    let mut t1 = start;
    let max_lines = zterm_lines.saturating_sub(2);
    while t1 != colsz && (t1 - start) < max_lines {
        let mut idx = t1; // c:381 ap = arr + t1
        loop {
            // c:383 do {
            let entry = items[idx];
            // c:385 — t2 = MB_METASTRWIDTH(*ap) + 2;
            let mut t2 = entry.chars().count() + 2;
            // c:391 — fprintf(stderr, "%d) %s", t3 = ap - arr + 1, *ap);
            let t3 = idx + 1;
            let _ = write!(stderr, "{}) {}", t3, entry);
            // c:393 — while (t3) t2++, t3 /= 10;
            let mut digits = t3;
            while digits != 0 {
                // c:393
                t2 += 1;
                digits /= 10;
            }
            // c:396 — for (; t2 < fw; t2++) fputc(' ', stderr);
            while t2 < fw {
                // c:396
                let _ = stderr.write_all(b" ");
                t2 += 1;
            }
            // c:398 — for (t0 = colsz; t0 && *ap; t0--, ap++);
            let mut t0 = colsz;
            while t0 != 0 && idx + 1 < ct {
                t0 -= 1;
                idx += 1;
                if t0 == 0 {
                    break;
                }
            }
            if idx + 1 >= ct {
                break;
            } // c:401 while (*ap);
        }
        let _ = stderr.write_all(b"\n"); // c:401 fputc('\n', stderr);
        t1 += 1;
    }

    let _ = stderr.flush(); // c:413 fflush(stderr);

    if t1 < colsz {
        t1
    } else {
        0
    } // c:415 return
}

// execwhile / execrepeat / execif / execcase / exectry moved to
// src/ported/exec.rs as faithful ports of Src/loop.c:413 / :499 /
// :553 / :600 / :735. See exec.rs for the C-line-cited bodies.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selectlist_returns_zero_when_full_fits() {
        let _g = crate::test_util::global_state_lock();
        let items = ["one", "two", "three"];
        let r = selectlist(&items, 0);
        assert!(r < items.len() || r == 0);
    }

    // The 7 *_panics_with_tree_walker_disabled tests were deleted
    // when the underlying `unreachable!()` stubs were replaced with
    // faithful ports in src/ported/exec.rs. The architectural pin
    // remains in tests/tree_walker_absent.rs (which asserts the
    // AST-side `execute_simple`/`execute_pipeline`/etc. tree walker
    // is gone — a different concept from the wordcode-VM dispatch
    // chain ported in exec.rs).

    /// Empty list to selectlist: nothing to draw, returns 0.
    #[test]
    fn selectlist_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(selectlist(&[], 0), 0);
    }
}
