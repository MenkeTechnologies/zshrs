//! Context save and restore for zshrs
//!
//! Direct port from zsh/Src/context.c
//!
//! This module provides a stack of saved contexts for history, lexer, and parser state.

use std::cell::RefCell;

/// Parts of context that can be saved/restored
pub const ZCONTEXT_HIST: u32 = 1;
pub const ZCONTEXT_LEX: u32 = 2;
pub const ZCONTEXT_PARSE: u32 = 4;

/// History state that gets pushed onto context stack
#[derive(Clone, Default)]
pub struct HistStack {
    pub curhist: usize,
    pub histsiz: usize,
    pub savehistsiz: usize,
}

/// Lexer state that gets pushed onto context stack
#[derive(Clone, Default)]
pub struct LexStack {
    pub tok: i32,
    pub tokstr: Option<String>,
    pub zsession: Option<String>,
}

/// Parser state that gets pushed onto context stack
#[derive(Clone, Default)]
pub struct ParseStack {
    pub ecused: usize,
    pub ecnpats: usize,
}

/// A saved context entry
#[derive(Clone, Default)]
pub struct ContextStack {
    pub hist_stack: HistStack,
    pub lex_stack: LexStack,
    pub parse_stack: ParseStack,
}

/// Context stack manager
pub struct ContextManager {
    stack: Vec<ContextStack>,
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextManager {
    pub fn new() -> Self {
        ContextManager { stack: Vec::new() }
    }

    /// Check if context stack is empty (at top level)
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Save some or all of current context
    pub fn save_partial(
        &mut self,
        parts: u32,
        hist: &HistStack,
        lex: &LexStack,
        parse: &ParseStack,
    ) {
        let mut ctx = ContextStack::default();

        if (parts & ZCONTEXT_HIST) != 0 {
            ctx.hist_stack = hist.clone();
        }
        if (parts & ZCONTEXT_LEX) != 0 {
            ctx.lex_stack = lex.clone();
        }
        if (parts & ZCONTEXT_PARSE) != 0 {
            ctx.parse_stack = parse.clone();
        }

        self.stack.push(ctx);
    }

    /// Save full context
    pub fn save(&mut self, hist: &HistStack, lex: &LexStack, parse: &ParseStack) {
        self.save_partial(
            ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE,
            hist,
            lex,
            parse,
        );
    }

    /// Restore some or all of context
    pub fn restore_partial(&mut self, parts: u32) -> Option<ContextStack> {
        let ctx = self.stack.pop()?;

        let mut result = ContextStack::default();
        if (parts & ZCONTEXT_HIST) != 0 {
            result.hist_stack = ctx.hist_stack;
        }
        if (parts & ZCONTEXT_LEX) != 0 {
            result.lex_stack = ctx.lex_stack;
        }
        if (parts & ZCONTEXT_PARSE) != 0 {
            result.parse_stack = ctx.parse_stack;
        }

        Some(result)
    }

    /// Restore full context
    pub fn restore(&mut self) -> Option<ContextStack> {
        self.restore_partial(ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE)
    }

    /// Get current stack depth
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}

thread_local! {
    static CONTEXT_STACK: RefCell<ContextManager> = RefCell::new(ContextManager::new());
}

/// Save context in full (global function)
pub fn zcontext_save(hist: &HistStack, lex: &LexStack, parse: &ParseStack) {
    CONTEXT_STACK.with(|cs| {
        cs.borrow_mut().save(hist, lex, parse);
    });
}

/// Save partial context (global function)
pub fn zcontext_save_partial(parts: u32, hist: &HistStack, lex: &LexStack, parse: &ParseStack) {
    CONTEXT_STACK.with(|cs| {
        cs.borrow_mut().save_partial(parts, hist, lex, parse);
    });
}

/// Restore full context (global function)
pub fn zcontext_restore() -> Option<ContextStack> {
    CONTEXT_STACK.with(|cs| cs.borrow_mut().restore())
}

/// Restore partial context (global function)
pub fn zcontext_restore_partial(parts: u32) -> Option<ContextStack> {
    CONTEXT_STACK.with(|cs| cs.borrow_mut().restore_partial(parts))
}

/// Check if we're at top level (no contexts saved)
pub fn zcontext_is_toplevel() -> bool {
    CONTEXT_STACK.with(|cs| cs.borrow().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_save_restore() {
        let mut mgr = ContextManager::new();

        let hist = HistStack {
            curhist: 100,
            histsiz: 1000,
            savehistsiz: 500,
        };
        let lex = LexStack {
            tok: 42,
            tokstr: Some("test".to_string()),
            zsession: None,
        };
        let parse = ParseStack {
            ecused: 10,
            ecnpats: 5,
        };

        mgr.save(&hist, &lex, &parse);
        assert_eq!(mgr.depth(), 1);

        let restored = mgr.restore().unwrap();
        assert_eq!(restored.hist_stack.curhist, 100);
        assert_eq!(restored.lex_stack.tok, 42);
        assert_eq!(restored.parse_stack.ecused, 10);
        assert_eq!(mgr.depth(), 0);
    }

    #[test]
    fn test_context_partial_save() {
        let mut mgr = ContextManager::new();

        let hist = HistStack {
            curhist: 50,
            histsiz: 500,
            savehistsiz: 250,
        };
        let lex = LexStack::default();
        let parse = ParseStack::default();

        mgr.save_partial(ZCONTEXT_HIST, &hist, &lex, &parse);

        let restored = mgr.restore_partial(ZCONTEXT_HIST).unwrap();
        assert_eq!(restored.hist_stack.curhist, 50);
    }

    #[test]
    fn test_nested_contexts() {
        let mut mgr = ContextManager::new();

        let hist1 = HistStack {
            curhist: 1,
            histsiz: 100,
            savehistsiz: 50,
        };
        let hist2 = HistStack {
            curhist: 2,
            histsiz: 200,
            savehistsiz: 100,
        };
        let lex = LexStack::default();
        let parse = ParseStack::default();

        mgr.save(&hist1, &lex, &parse);
        mgr.save(&hist2, &lex, &parse);

        assert_eq!(mgr.depth(), 2);

        let restored2 = mgr.restore().unwrap();
        assert_eq!(restored2.hist_stack.curhist, 2);

        let restored1 = mgr.restore().unwrap();
        assert_eq!(restored1.hist_stack.curhist, 1);

        assert!(mgr.is_empty());
    }
}
