//! context.c - context save and restore
//!
//! Port of Src/context.c
//!
//! This short file provides a home for the stack of saved contexts.
//! The actions for saving and restoring are encapsulated within
//! individual modules.

use crate::ported::zsh_h::hist_stack;
use crate::ported::zsh_h::{ZCONTEXT_HIST, ZCONTEXT_LEX, ZCONTEXT_PARSE};
use std::sync::Mutex;
use zshrs_parse::lex::{LexStack, ZshLexer};
use zshrs_parse::parse::{ParseStack, ZshParser};

/// Port of `struct context_stack` from Src/context.c:38-44.
#[allow(non_camel_case_types)]
pub struct context_stack {                                                   // c:38
    pub next: Option<Box<context_stack>>,                                    // c:39
    pub hist_stack: hist_stack,                                              // c:41
    pub lex_stack: LexStack,                                                 // c:42
    pub parse_stack: ParseStack,                                             // c:43
}

/// Port of `static struct context_stack *cstack` from Src/context.c:46.
static cstack: Mutex<Option<Box<context_stack>>> = Mutex::new(None);         // c:46

/// Port of `void zcontext_save_partial(int parts)` from Src/context.c:52.
///
/// Save some or all of current context. The C source reads from
/// hist.c / lex.c / parse.c file-statics; the Rust port takes the
/// owning `ZshLexer` and `ZshParser` because zshrs_parse doesn't
/// expose those subsystems as globals.
pub fn zcontext_save_partial(                                                // c:52
    parts: i32,
    lexer: Option<&mut ZshLexer<'_>>,
    parser: Option<&mut ZshParser<'_>>,
) {
    crate::ported::signals::queue_signals();                                 // c:56

    let mut cs = Box::new(context_stack {                                    // c:58
        next: None,
        hist_stack: hist_stack {
            histactive: 0, histdone: 0, stophist: 0, hlinesz: 0, defev: 0,
            hline: None, hptr: None, chwords: Vec::new(),
            chwordlen: 0, chwordpos: 0, csp: 0, hist_keep_comment: 0,
        },
        lex_stack: LexStack::default(),
        parse_stack: ParseStack::default(),
    });

    let mut head = cstack.lock().unwrap();

    let toplevel: i32 = if head.is_none() { 1 } else { 0 };                  // !cstack
    if (parts & ZCONTEXT_HIST) != 0 {                                        // c:60
        crate::ported::hist::hist_context_save(&mut cs.hist_stack, toplevel); // c:61
    }
    if (parts & ZCONTEXT_LEX) != 0 {                                         // c:63
        if let Some(lex) = lexer {                                           // c:64
            lex.lex_context_save(&mut cs.lex_stack);
        }
    }
    if (parts & ZCONTEXT_PARSE) != 0 {                                       // c:66
        if let Some(p) = parser {                                            // c:67
            p.parse_context_save(&mut cs.parse_stack);
        }
    }

    cs.next = head.take();                                                   // c:70
    *head = Some(cs);                                                        // c:71

    crate::ported::signals::unqueue_signals();                               // c:73
}

/// Port of `void zcontext_save(void)` from Src/context.c:80.
///
/// Save context in full.
pub fn zcontext_save(                                                        // c:80
    lexer: Option<&mut ZshLexer<'_>>,
    parser: Option<&mut ZshParser<'_>>,
) {
    zcontext_save_partial(                                                   // c:82
        ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE,
        lexer,
        parser,
    );
}

/// Port of `void zcontext_restore_partial(int parts)` from Src/context.c:89.
pub fn zcontext_restore_partial(                                             // c:89
    parts: i32,
    lexer: Option<&mut ZshLexer<'_>>,
    parser: Option<&mut ZshParser<'_>>,
) {
    let mut head = cstack.lock().unwrap();
    let mut cs = match head.take() {                                         // c:91
        Some(cs) => cs,
        None => {
            // DPUTS(!cstack, "BUG: zcontext_restore() without zcontext_save()"); // c:93
            return;
        }
    };

    crate::ported::signals::queue_signals();                                 // c:95
    *head = cs.next.take();                                                  // c:96 cstack = cstack->next
    let toplevel: i32 = if head.is_none() { 1 } else { 0 };                  // !cstack

    if (parts & ZCONTEXT_HIST) != 0 {                                        // c:98
        crate::ported::hist::hist_context_restore(&cs.hist_stack, toplevel); // c:99
    }
    if (parts & ZCONTEXT_LEX) != 0 {                                         // c:101
        if let Some(lex) = lexer {                                           // c:102
            lex.lex_context_restore(&mut cs.lex_stack);
        }
    }
    if (parts & ZCONTEXT_PARSE) != 0 {                                       // c:104
        if let Some(p) = parser {                                            // c:105
            p.parse_context_restore(&cs.parse_stack);
        }
    }

    drop(cs);                                                                // c:108 free(cs)

    crate::ported::signals::unqueue_signals();                               // c:110
}

/// Port of `void zcontext_restore(void)` from Src/context.c:117.
pub fn zcontext_restore(                                                     // c:117
    lexer: Option<&mut ZshLexer<'_>>,
    parser: Option<&mut ZshParser<'_>>,
) {
    zcontext_restore_partial(                                                // c:119
        ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE,
        lexer,
        parser,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_restore_balances_stack() {
        let mut lexer = ZshLexer::new("");
        let mut parser = ZshParser::new("");
        zcontext_save(Some(&mut lexer), Some(&mut parser));
        assert!(cstack.lock().unwrap().is_some());
        zcontext_restore(Some(&mut lexer), Some(&mut parser));
        assert!(cstack.lock().unwrap().is_none());
    }

    #[test]
    fn nested_saves_pop_lifo() {
        let mut lexer = ZshLexer::new("");
        let mut parser = ZshParser::new("");
        zcontext_save(Some(&mut lexer), Some(&mut parser));
        zcontext_save(Some(&mut lexer), Some(&mut parser));
        zcontext_restore(Some(&mut lexer), Some(&mut parser));
        assert!(cstack.lock().unwrap().is_some());
        zcontext_restore(Some(&mut lexer), Some(&mut parser));
        assert!(cstack.lock().unwrap().is_none());
    }

    #[test]
    fn restore_without_save_is_noop() {
        let mut lexer = ZshLexer::new("");
        let mut parser = ZshParser::new("");
        zcontext_restore(Some(&mut lexer), Some(&mut parser));
        assert!(cstack.lock().unwrap().is_none());
    }

    #[test]
    fn lex_save_restore_roundtrips_state() {
        let mut lexer = ZshLexer::new("echo hello");
        let mut parser = ZshParser::new("echo hello");
        // Mutate lexer state.
        lexer.dbparens = true;
        lexer.toklineno = 42;
        zcontext_save(Some(&mut lexer), Some(&mut parser));
        // Lexer should be reset to defaults after save.
        assert!(!lexer.dbparens);
        assert_eq!(lexer.toklineno, 0);
        zcontext_restore(Some(&mut lexer), Some(&mut parser));
        assert!(lexer.dbparens);
        assert_eq!(lexer.toklineno, 42);
    }
}
