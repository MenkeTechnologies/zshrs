//! Loop execution for zshrs
//!
//! Port from zsh/Src/loop.c (802 lines)
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
// Phase 3 bucket-2 (Arc<RwLock>) work. Currently mirrored as the
// AtomicI32 file-statics above (LOOP_DEPTH / CONT_FLAG / BREAK_LEVEL
// — names should be renamed to `LOOPS` / `CONTFLAG` / `BREAKS` to
// match C 1:1 in a follow-up).

/// Select-menu display.
/// Port of `selectlist()` from Src/loop.c:347 — formats the
/// numbered menu the C source uses for `select var in words`. Picks
/// columns automatically when `columns == 0`, mirroring the C
/// source's terminal-width auto-detection.
pub fn selectlist(items: &[String], prompt: &str, columns: usize) -> String {
    let mut output = String::new();
    let max_width = items.iter().map(|s| s.len()).max().unwrap_or(0);
    let item_width = max_width + 4; // number + ") " + padding
    let cols = if columns > 0 {
        columns
    } else {
        // Auto-detect columns based on terminal width
        let term_width = crate::utils::adjustcolumns();
        (term_width / item_width.max(1)).max(1)
    };

    for (i, item) in items.iter().enumerate() {
        let num = i + 1;
        let entry = format!("{:>2}) {:<width$}", num, item, width = max_width);
        output.push_str(&entry);

        if (i + 1) % cols == 0 || i + 1 == items.len() {
            output.push('\n');
        } else {
            output.push_str("  ");
        }
    }

    if !prompt.is_empty() {
        output.push_str(prompt);
    }

    output
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
    fn test_selectlist() {
        let items = vec!["one".into(), "two".into(), "three".into()];
        let output = selectlist(&items, "? ", 0);
        assert!(output.contains("1)"));
        assert!(output.contains("one"));
        assert!(output.contains("three"));
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

/// Port of `execfor()` from `Src/loop.c:50`. See module-level note:
/// fusevm bytecode replaces the tree walker; this entry is
/// `unreachable!()` to crash if regressed.
pub fn execfor(_do_exec: i32) -> i32 {                                   // c:50
    unreachable!("execfor: tree-walker disabled — fusevm lowers `for` in compile_zsh.rs")
}

/// Port of `execselect()` from `Src/loop.c:217`.
pub fn execselect(_do_exec: i32) -> i32 {                                // c:217
    unreachable!("execselect: tree-walker disabled — fusevm lowers `select` in compile_zsh.rs")
}

/// Port of `execwhile()` from `Src/loop.c:413`.
pub fn execwhile(_do_exec: i32) -> i32 {                                 // c:413
    unreachable!("execwhile: tree-walker disabled — fusevm lowers `while`/`until` in compile_zsh.rs")
}

/// Port of `execrepeat()` from `Src/loop.c:499`.
pub fn execrepeat(_do_exec: i32) -> i32 {                                // c:499
    unreachable!("execrepeat: tree-walker disabled — fusevm lowers `repeat` in compile_zsh.rs")
}

/// Port of `execif()` from `Src/loop.c:553`.
pub fn execif(_do_exec: i32) -> i32 {                                    // c:553
    unreachable!("execif: tree-walker disabled — fusevm lowers `if`/`elif`/`else` in compile_zsh.rs")
}

/// Port of `execcase()` from `Src/loop.c:600`.
pub fn execcase(_do_exec: i32) -> i32 {                                  // c:600
    unreachable!("execcase: tree-walker disabled — fusevm lowers `case` in compile_zsh.rs")
}

/// Port of `exectry()` from `Src/loop.c:735`.
pub fn exectry(_do_exec: i32) -> i32 {                                   // c:735
    unreachable!("exectry: tree-walker disabled — fusevm lowers `try`/`always` in compile_zsh.rs")
}
