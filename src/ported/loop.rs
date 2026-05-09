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

/// `for` loop tree-walker — port of `execfor()` from
/// Src/loop.c:50. Lowered to fusevm `LOOP_FOR_*` ops in the
/// compiler pass; this entry is a name-parity stub.
pub fn execfor(_do_exec: i32) -> i32 {
    0
}

/// `select` builtin tree-walker — port of `execselect()` from
/// Src/loop.c:217. Lowered to fusevm `SELECT_LOOP` ops; entry
/// kept for name parity.
pub fn execselect(_do_exec: i32) -> i32 {
    0
}

/// `while`/`until` tree-walker — port of `execwhile()` from
/// Src/loop.c:413. Lowered to fusevm `LOOP_WHILE` ops; entry
/// kept for name parity.
pub fn execwhile(_do_exec: i32) -> i32 {
    0
}

/// `repeat` tree-walker — port of `execrepeat()` from
/// Src/loop.c:499. Lowered to fusevm `REPEAT_LOOP` ops; entry
/// kept for name parity.
pub fn execrepeat(_do_exec: i32) -> i32 {
    0
}

/// `if`/`elif`/`else` tree-walker — port of `execif()` from
/// Src/loop.c:553. Lowered to fusevm `JMP_IF`/`JMP` ops; entry
/// kept for name parity.
pub fn execif(_do_exec: i32) -> i32 {
    0
}

/// `case` tree-walker — port of `execcase()` from
/// Src/loop.c:600. Lowered to fusevm `CASE_DISPATCH` ops; entry
/// kept for name parity.
pub fn execcase(_do_exec: i32) -> i32 {
    0
}

/// `try`/`always` tree-walker — port of `exectry()` from
/// Src/loop.c:735. Lowered to fusevm `TRY_ENTER`/`TRY_LEAVE`
/// ops; entry kept for name parity.
pub fn exectry(_do_exec: i32) -> i32 {
    0
}
