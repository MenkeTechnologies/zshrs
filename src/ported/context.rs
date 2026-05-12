//! context.c - context save and restore
//!
//! Port of Src/context.c
//!
//! This short file provides a home for the stack of saved contexts.
//! The actions for saving and restoring are encapsulated within
//! individual modules. After P7-P8 dissolved the ZshLexer and ZshParser structs into
//! free fns + thread_locals, the save/restore signatures simplified —
//! the lexer/parser parameters went away. Callers just call zcontext_*
//! and the underlying thread_local state is what's saved/restored.

use crate::ported::zsh_h::hist_stack;
use crate::ported::zsh_h::{ZCONTEXT_HIST, ZCONTEXT_LEX, ZCONTEXT_PARSE};
use std::sync::Mutex;
use crate::zsh_h::lex_stack;
use super::parse::{ParseStack};

/// Port of `struct context_stack` from Src/context.c:38-44.
#[allow(non_camel_case_types)]
// WARNING: FAKE IMPL RUST INVENTION — not in context.c
pub struct context_stack {                                                   // c:38
    pub next: Option<Box<context_stack>>,                                    // c:39
    pub hist_stack: hist_stack,                                              // c:41
    pub lex_stack: lex_stack,                                                 // c:42
    pub parse_stack: ParseStack,                                             // c:43
}

/// Port of `static struct context_stack *cstack` from Src/context.c:46.
static cstack: Mutex<Option<Box<context_stack>>> = Mutex::new(None);         // c:46

/// Port of `void zcontext_save_partial(int parts)` from Src/context.c:52.
#[allow(non_snake_case)]
pub fn zcontext_save_partial(parts: i32) {                                   // c:52
    crate::ported::signals::queue_signals();                                 // c:56

    let mut cs = Box::new(context_stack {                                    // c:58
        next: None,
        hist_stack: hist_stack {
            histactive: 0, histdone: 0, stophist: 0, hlinesz: 0, defev: 0,
            hline: None, hptr: None, chwords: Vec::new(),
            chwordlen: 0, chwordpos: 0, csp: 0, hist_keep_comment: 0,
        },
        lex_stack: lex_stack::default(),
        parse_stack: ParseStack::default(),
    });

    let mut head = cstack.lock().unwrap();

    let toplevel: i32 = if head.is_none() { 1 } else { 0 };                  // !cstack
    if (parts & ZCONTEXT_HIST) != 0 {                                        // c:60
        crate::ported::hist::hist_context_save(&mut cs.hist_stack, toplevel); // c:61
    }
    if (parts & ZCONTEXT_LEX) != 0 {                                         // c:63
        crate::ported::lex::lex_context_save(&mut cs.lex_stack);
    }
    if (parts & ZCONTEXT_PARSE) != 0 {                                       // c:66
        crate::ported::parse::parse_context_save(&mut cs.parse_stack);
    }

    cs.next = head.take();                                                   // c:70
    *head = Some(cs);                                                        // c:71

    crate::ported::signals::unqueue_signals();                               // c:73
}

/// Port of `void zcontext_save(void)` from Src/context.c:80.
pub fn zcontext_save() {                                                     // c:80
    zcontext_save_partial(ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE);
}

/// Port of `void zcontext_restore_partial(int parts)` from Src/context.c:89.
pub fn zcontext_restore_partial(parts: i32) {                                // c:89
    let mut head = cstack.lock().unwrap();
    let mut cs = match head.take() {                                         // c:91
        Some(cs) => cs,
        None => {
            return;
        }
    };

    crate::ported::signals::queue_signals();                                 // c:95
    *head = cs.next.take();                                                  // c:96
    let toplevel: i32 = if head.is_none() { 1 } else { 0 };

    if (parts & ZCONTEXT_HIST) != 0 {                                        // c:98
        crate::ported::hist::hist_context_restore(&cs.hist_stack, toplevel); // c:99
    }
    if (parts & ZCONTEXT_LEX) != 0 {                                         // c:101
        crate::ported::lex::lex_context_restore(&mut cs.lex_stack);
    }
    if (parts & ZCONTEXT_PARSE) != 0 {                                       // c:104
        crate::ported::parse::parse_context_restore(&cs.parse_stack);
    }

    drop(cs);                                                                // c:108

    crate::ported::signals::unqueue_signals();                               // c:110
}

/// Port of `void zcontext_restore(void)` from Src/context.c:117.
pub fn zcontext_restore() {                                                  // c:117
    zcontext_restore_partial(ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_cstack() {
        *cstack.lock().unwrap() = None;
    }

    #[test]
    fn save_restore_balances_stack() {
        reset_cstack();
        crate::ported::lex::lex_init("");
        zcontext_save();
        assert!(cstack.lock().unwrap().is_some());
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none());
    }

    #[test]
    fn nested_saves_pop_lifo() {
        reset_cstack();
        crate::ported::lex::lex_init("");
        zcontext_save();
        zcontext_save();
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_some());
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none());
    }

    #[test]
    fn restore_without_save_is_noop() {
        reset_cstack();
        crate::ported::lex::lex_init("");
        zcontext_restore();
        assert!(cstack.lock().unwrap().is_none());
    }

    #[test]
    fn lex_save_restore_roundtrips_state() {
        reset_cstack();
        crate::ported::lex::lex_init("echo hello");
        crate::ported::lex::LEX_DBPARENS.set(true);
        crate::ported::lex::set_toklineno(42);
        zcontext_save();
        assert!(crate::ported::lex::LEX_DBPARENS.with(|c| c.get()));
        assert_eq!(crate::ported::lex::toklineno(), 42);
        zcontext_restore();
        assert!(crate::ported::lex::LEX_DBPARENS.with(|c| c.get()));
        assert_eq!(crate::ported::lex::toklineno(), 42);
    }
}
