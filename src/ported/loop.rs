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

/// Loop state for the executor.
/// Port of the (`loops`, `breaks`, `contflag`) globals Src/loop.c
/// uses to coordinate `break`/`continue` with the loop bodies.
/// Bundling them into a struct gives us a single owner per executor
/// thread instead of file-static globals.
#[derive(Debug, Clone, Default)]
pub struct LoopState {
    /// Current nesting depth
    pub depth: i32,
    /// Break requested (and how many levels)
    pub breaks: i32,
    /// Continue requested (and how many levels)
    pub contflag: i32,
}

impl LoopState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enter a loop.
    /// Port of the `loops++` increment Src/loop.c performs at the
    /// top of every `execfor`/`execwhile`/etc. body before running
    /// any iterations.
    pub fn enter(&mut self) {
        self.depth += 1;
        LOOP_DEPTH.store(self.depth, Ordering::Relaxed);
    }

    /// Exit a loop.
    /// Port of the `loops--` decrement Src/loop.c performs at the
    /// end of each loop body. Also decrements pending `break` /
    /// `continue` levels so the next-outer loop sees them satisfied.
    pub fn exit(&mut self) {
        self.depth -= 1;
        if self.depth < 0 {
            self.depth = 0;
        }
        LOOP_DEPTH.store(self.depth, Ordering::Relaxed);

        // Decrement break/continue levels as we leave
        if self.breaks > 0 {
            self.breaks -= 1;
        }
        if self.contflag > 0 {
            self.contflag -= 1;
        }
        BREAK_LEVEL.store(self.breaks, Ordering::Relaxed);
        CONT_FLAG.store(self.contflag, Ordering::Relaxed);
    }

    /// Request break.
    /// Port of the `breaks = nlevels` write inside `bin_break()`
    /// (Src/builtin.c) — the C source clamps the level to the
    /// active loop depth.
    pub fn do_break(&mut self, levels: i32) {
        self.breaks = levels.min(self.depth);
        BREAK_LEVEL.store(self.breaks, Ordering::Relaxed);
    }

    /// Request continue.
    /// Port of the `contflag = nlevels` write inside `bin_break()`
    /// (Src/builtin.c) — same clamp to active loop depth.
    pub fn do_continue(&mut self, levels: i32) {
        self.contflag = levels.min(self.depth);
        CONT_FLAG.store(self.contflag, Ordering::Relaxed);
    }

    /// Check if break is active.
    /// Equivalent to the `breaks > 0` test inside `execfor`/etc.
    /// (Src/loop.c) that triggers loop-body teardown.
    pub fn should_break(&self) -> bool {
        self.breaks > 0
    }

    /// Check if continue is active.
    /// Equivalent to the `contflag > 0` test Src/loop.c uses to
    /// skip the rest of the iteration body.
    pub fn should_continue(&self) -> bool {
        self.contflag > 0
    }

    /// Check if we're inside any loop.
    /// Equivalent to the `loops > 0` test `bin_break()` uses to
    /// reject `break`/`continue` outside of a loop.
    pub fn in_loop(&self) -> bool {
        self.depth > 0
    }

    /// Reset break/continue (after handling).
    /// Port of the `contflag = 0` reset Src/loop.c performs at the
    /// top of each loop iteration — the body has consumed the
    /// continue request and is about to start fresh.
    pub fn reset_flow(&mut self) {
        self.contflag = 0;
        CONT_FLAG.store(0, Ordering::Relaxed);
    }

    /// Get current nesting depth.
    /// Returns the equivalent of the C source's `loops` value.
    pub fn current_depth(&self) -> i32 {
        self.depth
    }
}

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

/// `for` loop variable iteration helper.
/// Port of the word-list walk inside `execfor()` (Src/loop.c:50)
/// plus the integer-range walk inside `execfor`'s C-style branch.
/// The Rust struct exposes both shapes through a single `Iterator`
/// impl.
pub struct ForIterator {
    items: Vec<String>,
    pos: usize,
}

impl ForIterator {
    pub fn new(items: Vec<String>) -> Self {
        ForIterator { items, pos: 0 }
    }

    pub fn from_range(start: i64, end: i64, step: i64) -> Self {
        let mut items = Vec::new();
        let step = if step == 0 { 1 } else { step };
        if step > 0 {
            let mut i = start;
            while i <= end {
                items.push(i.to_string());
                i += step;
            }
        } else {
            let mut i = start;
            while i >= end {
                items.push(i.to_string());
                i += step;
            }
        }
        ForIterator { items, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Iterator for ForIterator {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        if self.pos < self.items.len() {
            let item = self.items[self.pos].clone();
            self.pos += 1;
            Some(item)
        } else {
            None
        }
    }
}

/// C-style `for` loop state (`(( init; cond; advance ))`).
/// Port of the `cs` (C-style) branch flags inside `execfor()`
/// (Src/loop.c:50). The C source threads init/cond/advance through
/// the bytecode walker; we keep a tiny init-done flag for the
/// equivalent first-iteration guard.
pub struct CForState {
    pub init_done: bool,
}

impl CForState {
    pub fn new() -> Self {
        CForState { init_done: false }
    }
}

impl Default for CForState {
    fn default() -> Self {
        Self::new()
    }
}

/// Try/always block state.
/// Port of the `try_errflag` / `try_retval` machinery
/// `exectry()` from Src/loop.c:735 saves and restores around the
/// `always { ... }` block. The C source uses globals here; we
/// scope them per executor instance.
#[derive(Debug, Clone, Default)]
pub struct TryState {
    pub in_try: bool,
    pub try_errflag: i32,
    pub try_retval: i32,
}

impl TryState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enter_try(&mut self) {
        self.in_try = true;
        self.try_errflag = 0;
        self.try_retval = 0;
    }

    pub fn exit_try(&mut self) {
        self.in_try = false;
    }

    pub fn set_error(&mut self, errflag: i32, retval: i32) {
        self.try_errflag = errflag;
        self.try_retval = retval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loop_state() {
        let mut state = LoopState::new();
        assert!(!state.in_loop());

        state.enter();
        assert!(state.in_loop());
        assert_eq!(state.current_depth(), 1);

        state.enter();
        assert_eq!(state.current_depth(), 2);

        state.exit();
        assert_eq!(state.current_depth(), 1);
        assert!(state.in_loop());

        state.exit();
        assert!(!state.in_loop());
    }

    #[test]
    fn test_break_continue() {
        let mut state = LoopState::new();
        state.enter();
        state.enter();

        state.do_break(1);
        assert!(state.should_break());

        state.exit();
        assert!(!state.should_break());
    }

    #[test]
    fn test_for_iterator() {
        let iter = ForIterator::new(vec!["a".into(), "b".into(), "c".into()]);
        let items: Vec<String> = iter.collect();
        assert_eq!(items, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_for_range() {
        let iter = ForIterator::from_range(1, 5, 1);
        let items: Vec<String> = iter.collect();
        assert_eq!(items, vec!["1", "2", "3", "4", "5"]);
    }

    #[test]
    fn test_selectlist() {
        let items = vec!["one".into(), "two".into(), "three".into()];
        let output = selectlist(&items, "? ", 0);
        assert!(output.contains("1)"));
        assert!(output.contains("one"));
        assert!(output.contains("three"));
    }

    #[test]
    fn test_try_state() {
        let mut state = TryState::new();
        assert!(!state.in_try);

        state.enter_try();
        assert!(state.in_try);

        state.set_error(1, 42);
        assert_eq!(state.try_errflag, 1);
        assert_eq!(state.try_retval, 42);

        state.exit_try();
        assert!(!state.in_try);
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
