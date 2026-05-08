//! Context save and restore for zshrs
//!
//! Direct port from zsh/Src/context.c
//!
//! This module provides a stack of saved contexts for history, lexer, and parser state.

use std::cell::RefCell;

/// Bit flags identifying which slices of context to save/restore.
/// Port of the `ZCONTEXT_*` macros from Src/zsh.h —
/// `zcontext_save_partial()` / `zcontext_restore_partial()` in
/// Src/context.c:52/89 take this bit set to control which
/// subsystem state they snapshot.
pub const ZCONTEXT_HIST: u32 = 1;
pub const ZCONTEXT_LEX: u32 = 2;
pub const ZCONTEXT_PARSE: u32 = 4;

/// History state slice pushed onto the context stack.
/// Port of the history-state fields `zcontext_save_partial()` from
/// Src/context.c:52 captures (curhist / histsiz / savehistsiz).
#[derive(Clone, Default)]
pub struct HistStack {
    pub curhist: usize,
    pub histsiz: usize,
    pub savehistsiz: usize,
}

/// Lexer state slice pushed onto the context stack.
/// Port of the lexer-state fields `zcontext_save_partial()` from
/// Src/context.c:52 captures (`tok`, `tokstr`, etc.) — same bit
/// `ZCONTEXT_LEX` controls them.
#[derive(Clone, Default)]
pub struct LexStack {
    pub tok: i32,
    pub tokstr: Option<String>,
    pub zsession: Option<String>,
}

/// Parser state slice pushed onto the context stack.
/// Port of the parser-state fields `zcontext_save_partial()` from
/// Src/context.c:52 captures (`ecused`, `ecnpats`) under
/// `ZCONTEXT_PARSE`.
#[derive(Clone, Default)]
pub struct ParseStack {
    pub ecused: usize,
    pub ecnpats: usize,
}

/// A single saved context entry.
/// Port of the per-entry shape on the C source's `zcontext_stack`
/// linked list — bundles all three subsystem slices so a single
/// push/pop captures everything `zcontext_save()` (Src/context.c:80)
/// snapshotted.
#[derive(Clone, Default)]
pub struct ContextStack {
    pub hist_stack: HistStack,
    pub lex_stack: LexStack,
    pub parse_stack: ParseStack,
}

/// Context stack manager.
/// Port of the global `zcontext_stack` linked list Src/context.c
/// keeps — a stack rather than a list since we never traverse it,
/// only push/pop.
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

    /// Save some or all of the current context.
    /// Port of `zcontext_save_partial()` from Src/context.c:52 —
    /// the C source allocates a fresh `zcontext_stack` node, fills
    /// the slices selected by `parts`, and pushes onto the stack.
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

    /// Save the full context.
    /// Port of `zcontext_save()` from Src/context.c:80 — wrapper
    /// over `save_partial(ZCONTEXT_HIST | ZCONTEXT_LEX |
    /// ZCONTEXT_PARSE, ...)`.
    pub fn save(&mut self, hist: &HistStack, lex: &LexStack, parse: &ParseStack) {
        self.save_partial(
            ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE,
            hist,
            lex,
            parse,
        );
    }

    /// Restore some or all of the saved context.
    /// Port of `zcontext_restore_partial()` from Src/context.c:89
    /// — pops the top stack node and copies back the slices
    /// selected by `parts`.
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

    /// Restore the full context.
    /// Port of `zcontext_restore()` from Src/context.c:117 —
    /// wrapper over `restore_partial(ZCONTEXT_HIST | ZCONTEXT_LEX
    /// | ZCONTEXT_PARSE)`.
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

/// Save the context in full (global entry point).
/// Port of `zcontext_save()` from Src/context.c:80 — the global
/// function the C source's eval/exec/parse paths call.
pub fn zcontext_save(hist: &HistStack, lex: &LexStack, parse: &ParseStack) {
    CONTEXT_STACK.with(|cs| {
        cs.borrow_mut().save(hist, lex, parse);
    });
}

/// Save partial context (global entry point).
/// Port of `zcontext_save_partial()` from Src/context.c:52.
pub fn zcontext_save_partial(parts: u32, hist: &HistStack, lex: &LexStack, parse: &ParseStack) {
    CONTEXT_STACK.with(|cs| {
        cs.borrow_mut().save_partial(parts, hist, lex, parse);
    });
}

/// Restore the full context (global entry point).
/// Port of `zcontext_restore()` from Src/context.c:117.
pub fn zcontext_restore() -> Option<ContextStack> {
    CONTEXT_STACK.with(|cs| cs.borrow_mut().restore())
}

/// Restore partial context (global entry point).
/// Port of `zcontext_restore_partial()` from Src/context.c:89.
pub fn zcontext_restore_partial(parts: u32) -> Option<ContextStack> {
    CONTEXT_STACK.with(|cs| cs.borrow_mut().restore_partial(parts))
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
