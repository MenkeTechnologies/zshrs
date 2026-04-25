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

/// Number of nested loops (from loop.c `loops`)
static LOOP_DEPTH: AtomicI32 = AtomicI32::new(0);

/// Continue flag / level (from loop.c `contflag`)
static CONT_FLAG: AtomicI32 = AtomicI32::new(0);

/// Break level (from loop.c `breaks`)
static BREAK_LEVEL: AtomicI32 = AtomicI32::new(0);

/// Loop state for the executor
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

    /// Enter a loop (from loop.c loops++)
    pub fn enter(&mut self) {
        self.depth += 1;
        LOOP_DEPTH.store(self.depth, Ordering::Relaxed);
    }

    /// Exit a loop (from loop.c loops--)
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

    /// Request break (from builtin break)
    pub fn do_break(&mut self, levels: i32) {
        self.breaks = levels.min(self.depth);
        BREAK_LEVEL.store(self.breaks, Ordering::Relaxed);
    }

    /// Request continue (from builtin continue)
    pub fn do_continue(&mut self, levels: i32) {
        self.contflag = levels.min(self.depth);
        CONT_FLAG.store(self.contflag, Ordering::Relaxed);
    }

    /// Check if break is active
    pub fn should_break(&self) -> bool {
        self.breaks > 0
    }

    /// Check if continue is active
    pub fn should_continue(&self) -> bool {
        self.contflag > 0
    }

    /// Check if we're inside any loop
    pub fn in_loop(&self) -> bool {
        self.depth > 0
    }

    /// Reset break/continue (after handling)
    pub fn reset_flow(&mut self) {
        self.contflag = 0;
        CONT_FLAG.store(0, Ordering::Relaxed);
    }

    /// Get current nesting depth
    pub fn current_depth(&self) -> i32 {
        self.depth
    }
}

/// Get global loop depth
pub fn loop_depth() -> i32 {
    LOOP_DEPTH.load(Ordering::Relaxed)
}

/// Get global break level
pub fn break_level() -> i32 {
    BREAK_LEVEL.load(Ordering::Relaxed)
}

/// Get global continue flag
pub fn cont_flag() -> i32 {
    CONT_FLAG.load(Ordering::Relaxed)
}

/// Select menu display (from loop.c selectlist)
///
/// Prints a numbered menu for `select var in words` loops.
/// Returns the formatted menu string.
pub fn selectlist(items: &[String], prompt: &str, columns: usize) -> String {
    let mut output = String::new();
    let max_width = items.iter().map(|s| s.len()).max().unwrap_or(0);
    let item_width = max_width + 4; // number + ") " + padding
    let cols = if columns > 0 {
        columns
    } else {
        // Auto-detect columns based on terminal width
        let term_width = crate::utils::get_term_width();
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

/// Parse select reply (from loop.c execselect)
///
/// Given the user's input and the item list, returns the selected item
/// or None if the input is invalid.
pub fn select_parse_reply(reply: &str, items: &[String]) -> Option<String> {
    let reply = reply.trim();
    if reply.is_empty() {
        return None;
    }

    // Try as number
    if let Ok(n) = reply.parse::<usize>() {
        if n >= 1 && n <= items.len() {
            return Some(items[n - 1].clone());
        }
    }

    None
}

/// For loop variable iteration helpers
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

/// C-style for loop state ((init; cond; advance))
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

/// Try/always block state (from loop.c exectry)
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
    fn test_select_parse() {
        let items = vec!["apple".into(), "banana".into(), "cherry".into()];
        assert_eq!(select_parse_reply("1", &items), Some("apple".to_string()));
        assert_eq!(select_parse_reply("3", &items), Some("cherry".to_string()));
        assert_eq!(select_parse_reply("0", &items), None);
        assert_eq!(select_parse_reply("4", &items), None);
        assert_eq!(select_parse_reply("", &items), None);
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
