//! Zsh lexical analyzer - Direct port from zsh/Src/lex.c
//!
//! This lexer tokenizes zsh shell input into a stream of tokens.
//! It handles all zsh-specific syntax including:
//! - Single/double/dollar quotes
//! - Command substitution $(...)  and `...`
//! - Arithmetic $((...))
//! - Parameter expansion ${...}
//! - Process substitution <(...) >(...)
//! - Here documents
//! - All redirection operators
//! - Comments
//! - Continuation lines

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

// =============================================================================
// Lexer-domain types — `enum lextok` (lextok), reserved-word table. These
// belong in lex.rs because they describe the lexer's output. Char tokens
// (POUND/STRING_TOK/INPAR/…), REDIR_* and COND_* constants live as flat
// `pub const`s in `super::zsh_h` per `Src/zsh.h:144-679` and are used from
// here directly — do NOT wrap them in Rust enums or sub-modules.
// =============================================================================

// Character tokens — port of `Src/zsh.h:144-224` `#define Pound … #define
// Marker`. Imported with the zsh_h.rs disambiguation (STRING_TOK → STRING_TOK,
// OUTANG_PROC → OUTANG_PROC) so the lex.rs body keeps the original C-style
// short names without colliding with `STRING_LEX` (the lextok=34 constant).
use crate::ported::zsh_h::{
    BANG, BAR, BNULL, BNULLKEEP, COMMA, DASH, DNULL, EQUALS, HAT, INANG, INBRACE, INBRACK, INPAR,
    INPARMATH, MARKER, META, NULARG, OUTANG, OUTANG_PROC, OUTBRACE, OUTBRACK, OUTPAR,
    OUTPARMATH, POUND, QSTRING, QTICK, QUEST, SNULL, STAR, STRING_TOK, TICK, TILDE,
};
use crate::zsh_h::lex_stack;
use crate::ztype_h::itok;

/// Char-level helper: map a single tokenised char back to its source
/// character (or None if the token has no plain equivalent — Snull/Dnull/
/// Bnull/etc.). Distinct from the string-level `untokenize(&str)` below.
pub fn untokenize_char(c: char) -> Option<char> {
    match c {
        POUND => Some('#'),
        STRING_TOK | QSTRING => Some('$'),
        HAT => Some('^'),
        STAR => Some('*'),
        INPAR | INPARMATH => Some('('),
        OUTPAR | OUTPARMATH => Some(')'),
        EQUALS => Some('='),
        BAR => Some('|'),
        INBRACE => Some('{'),
        OUTBRACE => Some('}'),
        INBRACK => Some('['),
        OUTBRACK => Some(']'),
        TICK | QTICK => Some('`'),
        INANG => Some('<'),
        OUTANG | OUTANG_PROC => Some('>'),
        QUEST => Some('?'),
        TILDE => Some('~'),
        COMMA => Some(','),
        DASH => Some('-'),
        BANG => Some('!'),
        SNULL | DNULL | BNULL | BNULLKEEP | NULARG | MARKER => None,
        _ => None,
    }
}

/// Port of `static const char ztokens[]` from `Src/lex.c:80`.
pub const ztokens: &str = "#$^*(())$=|{}[]`<>>?~`,-!'\"\\\\";

// `enum lextok` — port of `Src/zsh.h:304-371`. The full constant set
// (`NULLTOK`, `SEPER`, …, `TYPESET`) and the `lextok` type alias live
// in `super::zsh_h:198-262`. Re-export here so external callers can
// keep saying `lex::lextok` / `tokens::lextok` without reaching into
// `zsh_h::` directly. `IS_REDIROP()` (port of `Src/zsh.h:408`
// `#define IS_REDIROP`) lives in `zsh_h:318`.
pub use super::zsh_h::{
    lextok, AMPER, AMPERBANG, AMPOUTANG, BANG_TOK, BAR_TOK, BARAMP, CASE, COPROC, DAMPER, DBAR,
    DINANG, DINANGDASH, DINBRACK, DINPAR, DOLOOP, DONE, DOUTANG, DOUTANGAMP, DOUTANGAMPBANG,
    DOUTANGBANG, DOUTBRACK, DOUTPAR, DSEMI, ELIF, ELSE, ENDINPUT, ENVARRAY, ENVSTRING, ESAC, FI,
    FOR, FOREACH, FUNC, IF, INANG_TOK, INANGAMP, INBRACE_TOK, INOUTANG, INOUTPAR, INPAR_TOK,
    IS_REDIROP, LEXERR, NEWLIN, NOCORRECT, NULLTOK, OUTANG_TOK, OUTANGAMP, OUTANGAMPBANG,
    OUTANGBANG, OUTBRACE_TOK, OUTPAR_TOK, REPEAT, SELECT, SEMI, SEMIAMP, SEMIBAR, SEPER,
    STRING_LEX, THEN, TIME, TRINANG, TYPESET, UNTIL, WHILE, ZEND,
};

// RedirType / CondType — flat `REDIR_*` (`Src/zsh.h:377-408`) and
// `COND_*` (`Src/zsh.h:660-679`) constants already live in
// `super::zsh_h`. Do NOT wrap them in Rust enums here — the wrapper
// is a fake abstraction (no C counterpart).
//
// LX1_* / LX2_* — flat `#define`s in `Src/lex.c:371-405`. When the
// lexer's `gettok` body is faithfully ported it will reference those
// numeric values directly; no Rust enum wrapper needed.

// SPECCHARS / PATCHARS — port of `Src/zsh.h:228, 232`. Use
// `super::zsh_h::{SPECCHARS, PATCHARS}` directly; no duplicate here.
// IS_DASH() — port of `Src/zsh.h:242` `#define IS_DASH(x)`. Use
// `super::zsh_h::IS_DASH(c)` at call sites.

// Reserved-word table — the canonical port of `Src/hashtable.c:1076`
// `static struct reswd reswds[]` lives in `ported::hashtable::ReswdTable`
// (built in `ReswdTable::new()` at hashtable.rs:561 with the 31 entries
// from `reswds[]`). The lexer queries it via `reswdtab_lock()` from
// `check_reserved_word()` below; no duplicate table here.

#[cfg(test)]
mod tokens_tests {
    use crate::ported::hashtable::reswdtab_lock;
    use crate::ported::zsh_h::{
        BNULL, DINANG, DNULL, IF, IS_REDIROP, OUTANG_TOK, SNULL, STRING_LEX, THEN,
    };

    #[test]
    fn test_token_values() {
        assert_eq!(SNULL as u32, 0x9d);
        assert_eq!(DNULL as u32, 0x9e);
        assert_eq!(BNULL as u32, 0x9f);
    }

    #[test]
    fn test_reserved_words() {
        // Reserved-word lookup goes through the canonical `reswdtab`
        // (port of `Src/hashtable.c:1076 reswds[]`).
        let tab = reswdtab_lock().lock().unwrap();
        assert_eq!(tab.get("if").map(|r| r.token), Some(IF));
        assert_eq!(tab.get("then").map(|r| r.token), Some(THEN));
        assert!(tab.get("notakeyword").is_none());
    }

    #[test]
    fn test_redirop() {
        assert!(IS_REDIROP(OUTANG_TOK));
        assert!(IS_REDIROP(DINANG));
        assert!(!IS_REDIROP(IF));
        assert!(!IS_REDIROP(STRING_LEX));
    }
}

// =============================================================================
// End of inlined `tokens.rs`. Original lex.rs body follows.
// =============================================================================

// `lexflags` — port of `mod_export int lexflags;` (`Src/lex.c:151`).
// Carries the LEXFLAGS_ACTIVE/ZLE/COMMENTS_KEEP/COMMENTS_STRIP/NEWLINE
// bit flags from `Src/zsh.h:2293-2315`. The constants live in
// `super::zsh_h:2532-2537`; access via plain `&` / `|` ops, not a
// Rust struct.
pub use super::zsh_h::{
    LEXFLAGS_ACTIVE, LEXFLAGS_COMMENTS, LEXFLAGS_COMMENTS_KEEP, LEXFLAGS_COMMENTS_STRIP,
    LEXFLAGS_NEWLINE, LEXFLAGS_ZLE,
};

// `struct LexBuf` (fake Rust-only paraphrase) DELETED. The canonical
// port of `struct lexbufstate` (zsh.h:3069-3079) lives at
// `crate::ported::zsh_h::lexbufstate` with fields `{ptr: Option<String>,
// siz: i32, len: i32}` — same shape as C. ZshLexer now uses the
// canonical type directly. The convenience methods below are Rust-only
// helpers wrapping the flat operations C inlines at lex.c:451+ (`add`,
// etc.) — they're carried here as helpers rather than re-inlining
// ~50 lines of ptr/len/siz arithmetic across 24 call sites.
use crate::ported::zsh_h::lexbufstate;

// WARNING: NOT IN LEX.C — Rust-only convenience over C's flat lexbuf
// operations at lex.c:451+ (`add`, plus inline `*lexbuf.ptr = '\0'`
// terminator-writes, `lexbuf.len > oldlen` truncation loops, etc.).
// C operates on the file-static `lexbuf` global directly with raw
// ptr arithmetic; these methods package the equivalent ops on the
// owned-String buffer that zshrs's `lexbufstate.ptr: Option<String>`
// carries. Each method's body cites the C source location it mirrors.
impl lexbufstate {
    /// New lex buffer with the C-source initial alloc (lex.c:210
    /// `static struct lexbufstate lexbuf = { NULL, 256, 0 };`). ptr
    /// is initialized Some("") so subsequent operations can unwrap
    /// without a None check.
    pub(crate) fn new() -> Self {
        Self {
            ptr: Some(String::with_capacity(256)),
            siz: 256,
            len: 0,
        }
    }

    /// Mirrors C `add(int c)` at lex.c:451: push char, bump len,
    /// double siz on overflow. Skips the C `hrealloc` since Rust's
    /// String already manages capacity, but matches the doubling
    /// policy for parity with how downstream code observes `siz`.
    pub(crate) fn add(&mut self, c: char) {
        if let Some(p) = self.ptr.as_mut() {
            p.push(c);
            self.len = p.len() as i32;
            if self.len >= self.siz {
                self.siz *= 2;
                let want = self.siz as usize;
                let have = p.capacity();
                if want > have {
                    p.reserve(want - have);
                }
            }
        }
    }

    /// Reset buffer — clears ptr contents, zeros len, leaves siz.
    /// Mirrors lex.c:235 `tokstr = zshlextext = lexbuf.ptr = NULL;
    /// lexbuf.siz = 256;` partially (siz reset happens at the
    /// call site).
    pub(crate) fn clear(&mut self) {
        if let Some(p) = self.ptr.as_mut() {
            p.clear();
        }
        self.len = 0;
    }

    /// View buffer as &str. Empty if ptr is None.
    pub(crate) fn as_str(&self) -> &str {
        self.ptr.as_deref().unwrap_or("")
    }

    /// Length in chars (NOT bytes). Reads len field which `add()`
    /// keeps in sync; mirrors C `lexbuf.len`.
    pub(crate) fn buf_len(&self) -> usize {
        self.len as usize
    }

    /// Pop the last char. Mirrors C lex.c:524-526 hungetc-driven
    /// shrink: `lexbuf.len--; *--lexbuf.ptr = ...`.
    pub(crate) fn pop(&mut self) -> Option<char> {
        let c = self.ptr.as_mut().and_then(|p| p.pop());
        if c.is_some() {
            self.len -= 1;
        }
        c
    }
}

// Per-heredoc state — Rust-only AST-glue, NOT in lex.c. Canonical home
// is `src/extensions/heredoc_ast.rs`; re-exported here so existing
// `crate::lex::HereDoc` / `crate::parse::HereDocInfo` call sites keep
// resolving. Both die in Phase 9e (PORT_PLAN.md) when the wordcode
// port reinstates C's `struct heredocs` shape (zsh.h:1152) +
// `gethere()` deferred body collection.
pub use crate::extensions::heredoc_ast::HereDoc;

// =============================================================================
// ZshLexer state — thread-local file-statics matching zsh's lex.c file-statics.
// Each field maps to a `static` in `Src/lex.c` (or `Src/zsh.h` for the few
// `extern`-declared ones). Per-evaluator: each worker thread tokenizing its
// own input needs its own state (bucket-1 per PORT_PLAN.md).
// =============================================================================
thread_local! {
    /// Input source (owned). C uses input-stack `struct inputstack` +
    /// `hgetc()` (lex.c:input.c); zshrs P7 collapses to an owned String
    /// until the inputstack subsystem is ported.
    pub static LEX_INPUT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    pub static LEX_POS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub static LEX_UNGET_BUF: std::cell::RefCell<std::collections::VecDeque<char>>
        = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    /// `char *tokstr` (lex.c:170).
    pub static LEX_TOKSTR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// `enum lextok tok` (lex.c:180).
    pub static LEX_TOK: std::cell::Cell<lextok> = const { std::cell::Cell::new(ENDINPUT) };
    /// `int tokfd` (lex.c:191).
    pub static LEX_TOKFD: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
    /// `zlong toklineno` (lex.c:198).
    pub static LEX_TOKLINENO: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
    pub static LEX_LINENO: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
    /// `int lexstop` (lex.c:175).
    pub static LEX_LEXSTOP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// `int incmdpos` (lex.c:122 + extern in zsh.h).
    pub static LEX_INCMDPOS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// `int incond` (lex.c:127).
    pub static LEX_INCOND: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// In pattern context (RHS of == != =~ in [[ ]]) — zshrs extension
    /// over the bare incond state.
    pub static LEX_INCONDPAT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// `int incasepat` (lex.c:130).
    pub static LEX_INCASEPAT: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `int inredir` (lex.c:126).
    pub static LEX_INREDIR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Saved incmdpos from before a redirop/for/foreach/select. Mirrors
    /// `static int oldpos` in ctxtlex (lex.c:319).
    pub static LEX_OLDPOS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// `int infor` (lex.c:128).
    pub static LEX_INFOR: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `int inrepeat_` (lex.c:129).
    pub static LEX_INREPEAT: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `int intypeset` (lex.c:131).
    pub static LEX_INTYPESET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// `int dbparens` (lex.c:141).
    pub static LEX_DBPARENS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// `int noaliases` (lex.c:135).
    pub static LEX_NOALIASES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// `int nocorrect` (lex.c:144).
    pub static LEX_NOCORRECT: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `int nocomments` (lex.c:148).
    pub static LEX_NOCOMMENTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// `int lexflags` (lex.c:118).
    pub static LEX_LEXFLAGS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `int isfirstln` (lex.c:114).
    pub static LEX_ISFIRSTLN: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// `int isfirstch` (lex.c:116).
    pub static LEX_ISFIRSTCH: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Pending heredocs — Rust-only working set until P9c reinstates
    /// the C `struct heredocs` linked-list shape (zsh.h:1152).
    pub static LEX_HEREDOCS: std::cell::RefCell<Vec<HereDoc>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Heredoc-terminator-expected flag (0/1/2 for none / `<<` / `<<-`).
    pub static LEX_HEREDOC_PENDING: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    /// `struct lexbufstate lexbuf` (lex.c:210).
    pub static LEX_LEXBUF: std::cell::RefCell<lexbufstate> = const { std::cell::RefCell::new(
        lexbufstate { ptr: None, siz: 0, len: 0 }
    )};
    /// `int isnewlin` (lex.c:119).
    pub static LEX_ISNEWLIN: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// Last-error message — zshrs working state, not in C.
    pub static LEX_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// Safety counter for runaway iterations.
    pub static LEX_GLOBAL_ITERATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Safety counter for runaway recursion.
    pub static LEX_RECURSION_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// `int lex_add_raw` (lex.c:161).
    pub static LEX_LEX_ADD_RAW: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `struct lexbufstate lexbuf_raw` (lex.c:166).
    pub static LEX_LEXBUF_RAW: std::cell::RefCell<lexbufstate> = const { std::cell::RefCell::new(
        lexbufstate { ptr: None, siz: 0, len: 0 }
    )};
}

/// The Zsh Lexer.
///
/// Migration in progress (Phase 7 of PORT_PLAN.md): the file-scope
/// `LEX_*` thread-local statics above are the eventual home for every
/// field below — each one maps to a `static` in `Src/lex.c`. Methods
/// on `impl ZshLexer` currently read/write the struct fields; future
/// commits migrate them to read/write the thread-locals so the struct
/// can collapse to a unit type and external `lexer.X` accesses become
/// `lexer.X()` method calls or free-fn calls.
// All state lives in file-scope LEX_* thread_locals (above) matching
// C's lex.c file-statics. ZshLexer is now a unit struct — kept so call
// sites `let lexer = ZshLexer::new(input); lexer.zshlex(); ...` retain
// their familiar shape without leaking implementation. Methods on
// impl ZshLexer below are thin shims over the thread_local accesses.
//
// P7 migration history (PORT_PLAN.md Phase 7 batches):
//   isfirstch DELETED (was dead code).
//   batch-1: unget_buf, heredoc_pending, global_iterations,
//             recursion_depth → LEX_*.
//   batch-2: lexbuf, lexbuf_raw → LEX_LEXBUF, LEX_LEXBUF_RAW.
//   batch-3: lexstop, incondpat, oldpos, dbparens, noaliases,
//             nocorrect, nocomments, lexflags, isfirstln,
//             lex_add_raw → LEX_*.
//   batch-4: error, toklineno, tokfd, isnewlin, inrepeat, infor,
//             inredir, intypeset → LEX_* + accessor methods.
//   batch-5: lineno, incmdpos, incond, incasepat → LEX_*.
//   batch-6: heredocs Vec → LEX_HEREDOCS.
//   batch-7: tokstr, tok → LEX_TOKSTR, LEX_TOK.
//   batch-8: input, pos → LEX_INPUT (owned String) + LEX_POS;
//             ZshLexer becomes lifetime-free.
pub struct ZshLexer;

const MAX_LEXER_RECURSION: usize = 200;


impl ZshLexer {
    // ─── Accessor methods for migrated thread_local fields ───
    // These bridge external callers (parser/context) that read or
    // write the lexer state. Names match the former field identifiers.
    pub fn error(&self) -> Option<String> { LEX_ERROR.with_borrow(|e| e.clone()) }
    pub fn set_error(&self, v: Option<String>) { LEX_ERROR.with_borrow_mut(|e| *e = v); }
    pub fn toklineno(&self) -> u64 { LEX_TOKLINENO.get() }
    pub fn set_toklineno(&self, v: u64) { LEX_TOKLINENO.set(v); }
    pub fn tokfd(&self) -> i32 { LEX_TOKFD.get() }
    pub fn set_tokfd(&self, v: i32) { LEX_TOKFD.set(v); }
    pub fn isnewlin(&self) -> i32 { LEX_ISNEWLIN.get() }
    pub fn set_isnewlin(&self, v: i32) { LEX_ISNEWLIN.set(v); }
    pub fn inrepeat(&self) -> i32 { LEX_INREPEAT.get() }
    pub fn set_inrepeat(&self, v: i32) { LEX_INREPEAT.set(v); }
    pub fn infor(&self) -> i32 { LEX_INFOR.get() }
    pub fn set_infor(&self, v: i32) { LEX_INFOR.set(v); }
    pub fn inredir(&self) -> bool { LEX_INREDIR.get() }
    pub fn set_inredir(&self, v: bool) { LEX_INREDIR.set(v); }
    pub fn intypeset(&self) -> bool { LEX_INTYPESET.get() }
    pub fn set_intypeset(&self, v: bool) { LEX_INTYPESET.set(v); }
    pub fn lineno(&self) -> u64 { LEX_LINENO.get() }
    pub fn set_lineno(&self, v: u64) { LEX_LINENO.set(v); }
    pub fn incmdpos(&self) -> bool { LEX_INCMDPOS.get() }
    pub fn set_incmdpos(&self, v: bool) { LEX_INCMDPOS.set(v); }
    pub fn incond(&self) -> i32 { LEX_INCOND.get() }
    pub fn set_incond(&self, v: i32) { LEX_INCOND.set(v); }
    pub fn incasepat(&self) -> i32 { LEX_INCASEPAT.get() }
    pub fn set_incasepat(&self, v: i32) { LEX_INCASEPAT.set(v); }
    /// Pending-heredocs accessors. The Vec lives in LEX_HEREDOCS;
    /// these helpers package the common operations so callers don't
    /// touch the thread_local directly.
    pub fn heredocs_take(&self) -> Vec<HereDoc> {
        LEX_HEREDOCS.with_borrow_mut(|v| std::mem::take(v))
    }
    pub fn heredocs_set(&self, v: Vec<HereDoc>) {
        LEX_HEREDOCS.with_borrow_mut(|c| *c = v);
    }
    pub fn heredocs_clear(&self) {
        LEX_HEREDOCS.with_borrow_mut(|v| v.clear());
    }
    pub fn heredocs_is_empty(&self) -> bool {
        LEX_HEREDOCS.with_borrow(|v| v.is_empty())
    }
    pub fn heredocs_len(&self) -> usize {
        LEX_HEREDOCS.with_borrow(|v| v.len())
    }
    pub fn heredocs_clone(&self) -> Vec<HereDoc> {
        LEX_HEREDOCS.with_borrow(|v| v.clone())
    }
    pub fn heredocs_push(&self, h: HereDoc) {
        LEX_HEREDOCS.with_borrow_mut(|v| v.push(h));
    }
    /// `char *tokstr` accessors — direct port of lex.c:170 file-static.
    pub fn tokstr(&self) -> Option<String> {
        LEX_TOKSTR.with_borrow(|t| t.clone())
    }
    pub fn set_tokstr(&self, v: Option<String>) {
        LEX_TOKSTR.with_borrow_mut(|t| *t = v);
    }
    pub fn tokstr_take(&self) -> Option<String> {
        LEX_TOKSTR.with_borrow_mut(|t| t.take())
    }
    pub fn tokstr_is_some(&self) -> bool {
        LEX_TOKSTR.with_borrow(|t| t.is_some())
    }
    pub fn tokstr_is_none(&self) -> bool {
        LEX_TOKSTR.with_borrow(|t| t.is_none())
    }
    pub fn tokstr_eq(&self, s: &str) -> bool {
        LEX_TOKSTR.with_borrow(|t| t.as_deref() == Some(s))
    }
    /// `enum lextok tok` accessors — direct port of lex.c:180 file-static.
    pub fn tok(&self) -> lextok { LEX_TOK.get() }
    pub fn set_tok(&self, v: lextok) { LEX_TOK.set(v); }
    pub fn pos(&self) -> usize { LEX_POS.get() }
    pub fn set_pos(&self, v: usize) { LEX_POS.set(v); }
    /// Slice the input source from `start..end` — used by parse.rs to
    /// capture function body source text. Returns None if out-of-range.
    pub fn input_slice(&self, start: usize, end: usize) -> Option<String> {
        LEX_INPUT.with_borrow(|s| s.get(start..end).map(String::from))
    }

    /// Create a new lexer for the given input
    pub fn new(input: &str) -> Self {
        // Reset migrated thread-locals so a fresh lexer instance
        // starts from a clean slate (same as the C source's
        // file-static initializers in lex.c).
        LEX_UNGET_BUF.with_borrow_mut(|b| b.clear());
        LEX_HEREDOC_PENDING.set(0);
        LEX_GLOBAL_ITERATIONS.set(0);
        LEX_RECURSION_DEPTH.set(0);
        LEX_LEXBUF.with_borrow_mut(|b| *b = lexbufstate::new());
        LEX_LEXBUF_RAW.with_borrow_mut(|b| *b = lexbufstate::new());
        // P7-batch-3 fields: reset to their C-source initial values.
        LEX_LEXSTOP.set(false);
        LEX_INCONDPAT.set(false);
        LEX_OLDPOS.set(true);
        LEX_DBPARENS.set(false);
        LEX_NOALIASES.set(false);
        LEX_NOCORRECT.set(0);
        LEX_NOCOMMENTS.set(false);
        LEX_LEXFLAGS.set(0);
        LEX_ISFIRSTLN.set(true);
        LEX_LEX_ADD_RAW.set(0);
        // P7-batch-4 resets.
        LEX_TOKFD.set(-1);
        LEX_TOKLINENO.set(1);
        LEX_INREDIR.set(false);
        LEX_INFOR.set(0);
        LEX_INREPEAT.set(0);
        LEX_INTYPESET.set(false);
        LEX_ISNEWLIN.set(0);
        LEX_ERROR.with_borrow_mut(|e| *e = None);
        // P7-batch-5 resets.
        LEX_LINENO.set(1);
        LEX_INCMDPOS.set(true);
        LEX_INCOND.set(0);
        LEX_INCASEPAT.set(0);
        // P7-batch-6 reset.
        LEX_HEREDOCS.with_borrow_mut(|v| v.clear());
        // P7-batch-7 resets.
        LEX_TOKSTR.with_borrow_mut(|t| *t = None);
        LEX_TOK.set(ENDINPUT);
        // P7-batch-8: input + pos.
        LEX_INPUT.with_borrow_mut(|s| { s.clear(); s.push_str(input); });
        LEX_POS.set(0);
        ZshLexer
    }

    /// Append a char to the raw-input capture buffer. Direct port of
    /// zsh/Src/lex.c:2024-2039 `zshlex_raw_add`. Called from hgetc
    /// when `lex_add_raw` is nonzero so cmd-sub bodies (`$(...)`,
    /// `<(...)`, `>(...)`) can be replayed verbatim without re-lexing.
    pub fn zshlex_raw_add(&mut self, c: char) {
        // lex.c:2027-2028 — guard on lex_add_raw flag.
        if LEX_LEX_ADD_RAW.get() == 0 {
            return;
        }
        // lex.c:2030-2038 — append to lexbuf_raw. The C source manages
        // explicit ptr/len/siz with hrealloc; Rust's String handles
        // resize automatically.
        LEX_LEXBUF_RAW.with_borrow_mut(|b| b.add(c));
    }

    /// Run alias / reserved-word expansion on the just-lexed token.
    /// Direct port of zsh/Src/lex.c:1949-2021 `exalias`. Returns true
    /// if an alias was injected (the caller's loop should re-run
    /// gettok to consume the injected text).
    ///
    /// C source flow:
    ///   1. Spell-correct (lex.c:1958-1962) — disabled in zshrs.
    ///   2. If tokstr is None: set lextext from `tokstrings[tok]` and
    ///      checkalias against that (lex.c:1964-1969).
    ///   3. Otherwise: untokenize tokstr into a working copy (lex.c:
    ///      1971-1980).
    ///   4. ZLE word-tracking: call gotword() if LEXFLAGS_ZLE
    ///      (lex.c:1982-1991).
    ///   5. STRING_TOK tokens: try checkalias, then reservation lookup
    ///      (lex.c:1993-2015).
    ///   6. Clear inalmore (lex.c:2016).
    ///
    /// Direct port of `exalias(void)` at `Src/lex.c:1953`. No
    /// parameters — reads global `aliastab`/`sufaliastab`/`reswdtab`
    /// directly, mirroring C.
    pub fn exalias(&mut self) -> bool {
        // lex.c:1957 — `hwend()` ends the history-word region. zshrs's
        // history layer doesn't track per-word boundaries here; no-op.

        // lex.c:1958-1962 — spell correction via spckword. zshrs
        // doesn't implement spell correction yet; documented divergence.

        // lex.c:1964-1969 — bare-token path (no tokstr).
        if self.tokstr_is_none() {
            // lex.c:1965 — `zshlextext = tokstrings[tok];` — for tokens
            // like SEMI/AMPER/etc. the canonical text comes from a
            // static table.
            if self.tok() == NEWLIN {
                return false;
            }
            // Use punctuation-token text; unknown tokens skip alias.
            let text = match self.tok() {
                SEMI => ";",
                AMPER => "&",
                BAR_TOK => "|",
                _ => return false,
            };
            return self.check_alias(text);
        }

        let tokstr = self.tokstr().unwrap();
        // lex.c:1973-1980 — untokenize: convert the lexer's internal
        // tokenized form (Pound..ztokens shifts) into the literal
        // shell text. Call the global helper.
        let lextext = if has_token(&tokstr) {
            untokenize(&tokstr)
        } else {
            tokstr.clone()
        };

        // lex.c:1982-1991 — ZLE word-tracking for completion.
        if LEX_LEXFLAGS.get() & LEXFLAGS_ZLE != 0 {
            let zp = LEX_LEXFLAGS.get();
            self.gotword();
            // lex.c:1986-1990 — if gotword cleared lexflags, the cursor
            // word has been reached; abort exalias so completion can
            // capture the partial token unchanged.
            // lex.c:1986 — `(zp & LEXFLAGS_ZLE) && !lexflags` — gotword
            // fully cleared lexflags (not just ZLE) when the cursor word
            // was reached.
            if (zp & LEXFLAGS_ZLE) != 0 && LEX_LEXFLAGS.get() == 0 {
                return false;
            }
        }

        // lex.c:1993-2015 — STRING_TOK-token alias / reswd check.
        if self.tok() == STRING_LEX {
            // lex.c:1995 — `checkalias()`. POSIX-aliases gate skipped
            // here (zshrs doesn't have the option flag wired).
            if self.check_alias(&lextext) {
                return true;
            }

            // lex.c:2002-2009 — reserved-word lookup. Fires when in
            // command position OR when the text is bare `}` and
            // IGNOREBRACES is unset (so `}` ends a brace block).
            if LEX_INCMDPOS.get() || lextext == "}" {
                // lex.c:2002 — `(rw = (Reswd) reswdtab->getnode(reswdtab, tokstr))`
                let rw_tok: Option<lextok> = {
                    let guard = crate::ported::hashtable::reswdtab_lock()
                        .lock()
                        .expect("reswdtab poisoned");
                    guard.get(&lextext).map(|r| r.token)
                };
                if let Some(rwtok) = rw_tok {
                    self.set_tok(rwtok);
                    if rwtok == REPEAT {
                        LEX_INREPEAT.set(1);
                    }
                    if rwtok == DINBRACK {
                        LEX_INCOND.set(1);
                    }
                }
            } else if LEX_INCOND.get() > 0 && lextext == "]]" {
                // lex.c:2010-2012 — `]]` closes the cond expression.
                self.set_tok(DOUTBRACK);
                LEX_INCOND.set(0);
            } else if LEX_INCOND.get() == 1 && lextext == "!" {
                // lex.c:2013-2014 — `!` inside `[[ ]]` is the BANG
                // negation, not a literal.
                self.set_tok(BANG_TOK);
            }
        }

        // lex.c:2016 — `inalmore = 0;` — alias-more flag clears after
        // any non-alias token.
        // (zshrs's lexer doesn't have inalmore yet — added here would
        // require gettok to track when an alias-pushed token has more
        // text after it. Documented divergence.)

        false
    }

    /// Direct port of `checkalias(void)` at `Src/lex.c:1902`. No
    /// parameters in C — reads `aliastab`/`sufaliastab` directly.
    /// zshrs threads `lextext` in because it's already untokenized at
    /// the call site; C re-derives it from `zshlextext`. Returns true
    /// if the lookup matched (regular or suffix alias) AND the alias
    /// text was successfully injected back into the input stream for
    /// re-lexing.
    fn check_alias(&mut self, lextext: &str) -> bool {
        // lex.c:1906-1907 — guard on null lextext.
        if lextext.is_empty() {
            return false;
        }

        // lex.c:1909-1911 — guard: alias expansion is disabled, or
        // POSIX aliases require the token to be a STRING_TOK and not a
        // reserved word.
        if LEX_NOALIASES.get() {
            return false;
        }

        // lex.c:1914-1933 — regular alias lookup. C: `an = (Alias)
        // aliastab->getnode(aliastab, zshlextext);`
        let alias_clone: Option<crate::ported::zsh_h::alias> = {
            let guard = crate::ported::hashtable::aliastab_lock()
                .lock()
                .expect("aliastab poisoned");
            guard.get(lextext).cloned()
        };
        if let Some(alias) = alias_clone {
            let is_global =
                (alias.node.flags & crate::ported::zsh_h::ALIAS_GLOBAL) != 0;
            if alias.inuse == 0
                && (is_global || (LEX_INCMDPOS.get() && self.tok() == STRING_LEX))
            {
                // lex.c:1918-1927 — if the next char isn't blank,
                // insert a space so the alias body can't accidentally
                // join the following word.
                if !LEX_LEXSTOP.get() {
                    if let Some(c) = self.peek() {
                        if !Self::is_blank(c) {
                            self.inject_alias_text(" ");
                        }
                    }
                }
                // lex.c:1928 — `inpush(an->text, INP_ALIAS, an);`
                self.inject_alias_text(&alias.text);
                // lex.c:1929 — `an->inuse = 1;` (set on the live node).
                let mut guard = crate::ported::hashtable::aliastab_lock()
                    .lock()
                    .expect("aliastab poisoned");
                if let Some(a) = guard.get_mut(lextext) {
                    a.inuse = 1;
                }
                drop(guard);
                LEX_LEXSTOP.set(false);
                return true;
            }
        }

        // lex.c:1934-1943 — suffix-alias lookup. The token must end
        // with `.SUFFIX`, the suffix name must be a registered
        // suffix-alias, AND the lexer must be in command position.
        if LEX_INCMDPOS.get() {
            if let Some(dot_pos) = lextext.rfind('.') {
                if dot_pos > 0 && dot_pos + 1 < lextext.len() {
                    let suffix = &lextext[dot_pos + 1..];
                    let alias_clone: Option<crate::ported::zsh_h::alias> = {
                        let guard =
                            crate::ported::hashtable::sufaliastab_lock()
                                .lock()
                                .expect("sufaliastab poisoned");
                        guard.get(suffix).cloned()
                    };
                    if let Some(alias) = alias_clone {
                        if alias.inuse == 0 {
                            // lex.c:1938-1940 — push three things in
                            // reverse: the alias text, a space, then
                            // the original word.
                            self.inject_alias_text(&alias.text);
                            self.inject_alias_text(" ");
                            self.inject_alias_text(lextext);
                            // lex.c:1941 — `an->inuse = 1;` on the
                            // suffix-alias node.
                            let mut guard = crate::ported::hashtable::sufaliastab_lock()
                                .lock()
                                .expect("sufaliastab poisoned");
                            if let Some(a) = guard.get_mut(suffix) {
                                a.inuse = 1;
                            }
                            drop(guard);
                            LEX_LEXSTOP.set(false);
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Push alias text back into the input stream so the lexer
    /// re-reads it. Equivalent to zsh's `inpush(text, INP_ALIAS, an)`
    /// at lex.c:1928,1938,1940. zshrs uses the existing `unget_buf`
    /// (a VecDeque<char>) to inject chars in reverse order so the
    /// next hgetc consumes them first.
    fn inject_alias_text(&mut self, text: &str) {
        // Insert at front in reverse so the first char of `text`
        // comes out first.
        LEX_UNGET_BUF.with_borrow_mut(|buf| {
            for c in text.chars().rev() {
                buf.push_front(c);
            }
        });
    }

    /// Pop the last char from the raw-input capture buffer. Direct
    /// port of zsh/Src/lex.c:2042-2049 `zshlex_raw_back`. Called when
    /// the lexer ungets a char that was just captured raw — the raw
    /// buffer must mirror the live input so this undoes the last add.
    pub fn zshlex_raw_back(&mut self) {
        // lex.c:2045-2046 — guard.
        if LEX_LEX_ADD_RAW.get() == 0 {
            return;
        }
        // lex.c:2047-2048 — `lexbuf_raw.ptr--; lexbuf_raw.len--;`
        LEX_LEXBUF_RAW.with_borrow_mut(|b| b.pop());
    }

    /// Mark the current raw-buffer offset (for restore later). Direct
    /// port of zsh/Src/lex.c:2052-2058 `zshlex_raw_mark`. Returns
    /// `len + offset` so callers can restore via `back_to_mark`.
    pub fn zshlex_raw_mark(&self, offset: i64) -> i64 {
        // lex.c:2055-2056 — guard.
        if LEX_LEX_ADD_RAW.get() == 0 {
            return 0;
        }
        // lex.c:2057 — `return lexbuf_raw.len + offset;`
        (LEX_LEXBUF_RAW.with_borrow(|b| b.buf_len()) as i64) + offset
    }

    /// Restore raw-buffer offset to a previously-saved mark. Direct
    /// port of zsh/Src/lex.c:2061-2068 `zshlex_raw_back_to_mark`.
    /// Truncates the raw buffer to `mark` bytes — undoes any captures
    /// since the mark was taken (used when a speculative parse fails
    /// and the lexer rolls back).
    pub fn zshlex_raw_back_to_mark(&mut self, mark: i64) {
        // lex.c:2064-2065 — guard.
        if LEX_LEX_ADD_RAW.get() == 0 {
            return;
        }
        // lex.c:2066-2067 — `lexbuf_raw.ptr = tokstr_raw + mark;
        // lexbuf_raw.len = mark;` — String::truncate handles both.
        let m = mark.max(0) as usize;
        LEX_LEXBUF_RAW.with_borrow_mut(|b| {
            if let Some(p) = b.ptr.as_mut() {
                p.truncate(m);
            }
            b.len = m as i32;
        });
    }

    /// Take the captured raw-input buffer, clearing it. Useful for
    /// callers that need the literal command-sub body after lexing
    /// (e.g. compile-time string capture for `$(...)`).
    pub fn take_raw_buf(&mut self) -> String {
        LEX_LEXBUF_RAW.with_borrow_mut(|b| {
            let out = b.ptr.take().unwrap_or_default();
            b.ptr = Some(String::with_capacity(256));
            b.len = 0;
            out
        })
    }

    /// zsh/Src/lex.c:215-239 `lex_context_save`. After save, the lexer
    /// is in a clean state suitable for parsing a nested input (command
    /// substitution body, here-doc terminator, eval'd string).
    pub fn lex_context_save(&mut self, ls: &mut lex_stack) {
        // lex.c:220-233 — copy live state into the stack. ZshLexer
        // mirrors the C fields with idiomatic Rust types (bool / u64);
        // canonical `lex_stack` (zsh_h.rs:1045, port of zsh.h:3082) is
        // i32 / i64 / lexbufstate — convert at the boundary.
        ls.dbparens = LEX_DBPARENS.get() as i32;
        ls.isfirstln = LEX_ISFIRSTLN.get() as i32;
        // isfirstch — field deleted (was unused). Stash 0; canonical
        // C tracks this for spell-correction which zshrs doesn't run.
        ls.isfirstch = 0;
        ls.lexflags = LEX_LEXFLAGS.get();
        ls.tok = self.tok();
        ls.tokstr = self.tokstr_take();
        LEX_LEXBUF.with_borrow_mut(|b| {
            ls.lexbuf.ptr = b.ptr.take();
            ls.lexbuf.siz = b.siz;
            ls.lexbuf.len = b.len;
        });
        ls.lexstop = LEX_LEXSTOP.get() as i32;
        ls.toklineno = LEX_TOKLINENO.get() as i64;
        // zshlextext / lex_add_raw / tokstr_raw / lexbuf_raw — these
        // are real `struct lex_stack` fields (zsh.h:3089-3093) but the
        // current ZshLexer doesn't track those globals yet; they stay
        // at their `Default` (None / 0) until the raw-token capture
        // path is ported.

        // lex.c:235-238 — reset live state to defaults so a nested
        // parse starts from a clean slate. tokstr/lexbuf are zeroed,
        // lexbuf.siz reset to 256 (the C-source initial alloc).
        self.set_tokstr(None);
        LEX_LEXBUF.with_borrow_mut(|b| {
            b.ptr = Some(String::with_capacity(256));
            b.siz = 256;
            b.len = 0;
        });
    }

    /// zsh/Src/lex.c:244-262 `lex_context_restore`. Inverse of
    /// `lex_context_save`. Called after the nested parse completes.
    pub fn lex_context_restore(&mut self, ls: &mut lex_stack) {
        // lex.c:249-261 — copy stack state back into live fields.
        LEX_DBPARENS.set(ls.dbparens != 0);
        LEX_ISFIRSTLN.set(ls.isfirstln != 0);
        // isfirstch — field deleted (was unused); discard ls.isfirstch.
        let _ = ls.isfirstch;
        LEX_LEXFLAGS.set(ls.lexflags);
        self.set_tok(ls.tok);
        self.set_tokstr(ls.tokstr.take());
        LEX_LEXBUF.with_borrow_mut(|b| {
            b.ptr = Some(ls.lexbuf.ptr.take().unwrap_or_default());
            b.siz = ls.lexbuf.siz;
            b.len = ls.lexbuf.len;
        });
        LEX_LEXSTOP.set(ls.lexstop != 0);
        LEX_TOKLINENO.set(ls.toklineno as u64);
    }

    /// Initialize lexical state. Direct port of zsh/Src/lex.c:440-445
    /// `lexinit`. Resets dbparens / nocorrect / lexstop and sets `tok`
    /// to ENDINPUT so the next gettok starts from a known baseline.
    /// Note: the constructor `Self::new` already sets equivalent
    /// defaults; this method exists for the rare case a caller wants
    /// to recycle a `ZshLexer` across multiple input strings.
    pub fn lexinit(&mut self) {
        // lex.c:443 — `nocorrect = dbparens = lexstop = 0;`
        LEX_NOCORRECT.set(0);
        LEX_DBPARENS.set(false);
        LEX_LEXSTOP.set(false);
        // lex.c:444 — `tok = ENDINPUT;`
        self.set_tok(ENDINPUT);
    }

    /// Check recursion depth; returns true if exceeded
    #[inline]
    fn check_recursion(&mut self) -> bool {
        if LEX_RECURSION_DEPTH.get() > MAX_LEXER_RECURSION {
            LEX_ERROR.with_borrow_mut(|e| *e = Some("lexer exceeded max recursion depth".to_string()));
            LEX_LEXSTOP.set(true);
            true
        } else {
            false
        }
    }

    /// Check and increment global iteration counter; returns true if limit exceeded
    /// Soft cap on `hgetc` invocations — an infinite-loop tripwire.
    /// Real-world scripts: zinit.zsh ~5K lines / ~200KB, p10k's
    /// internal/p10k.zsh ~10K lines / ~360KB, the user's daily-driver
    /// `.zshrc` + zpwr stack collectively crosses 1M+ chars per shell
    /// invocation. The previous 50K cap was tripped by p10k by line
    /// 1277 (well below its actual 10K-line size). 100M chars handles
    /// every reasonable script while still bailing out of a real
    /// runaway lexer state machine.
    const LEXER_HGETC_CAP: u64 = 100_000_000;

    #[inline]
    fn check_iterations(&mut self) -> bool {
        let next = LEX_GLOBAL_ITERATIONS.get() + 1;
        LEX_GLOBAL_ITERATIONS.set(next);
        if next as u64 > Self::LEXER_HGETC_CAP {
            LEX_ERROR.with_borrow_mut(|e| *e = Some(format!(
                "lexer exceeded {} hgetc iterations — possible infinite loop",
                Self::LEXER_HGETC_CAP
            )));
            LEX_LEXSTOP.set(true);
            self.set_tok(LEXERR);
            true
        } else {
            false
        }
    }

    /// Get next character from input
    fn hgetc(&mut self) -> Option<char> {
        if self.check_iterations() {
            return None;
        }

        // Re-read from unget_buf: increment lineno on `\n` HERE
        // too. hungetc() decremented lineno when the char was put
        // back; without a matching increment on the way out, every
        // `\n` that's ungetted-then-reread leaves lineno
        // permanently one short. Symptom: $LINENO stuck at 1 in
        // every script statement because the parser ungets the
        // separating newline once between statements.
        if let Some(c) = LEX_UNGET_BUF.with_borrow_mut(|b| b.pop_front()) {
            if c == '\n' {
                LEX_LINENO.set(LEX_LINENO.get() + 1);
            }
            return Some(c);
        }

        let c = LEX_INPUT.with_borrow(|s| s[LEX_POS.get()..].chars().next())?;
        LEX_POS.set(LEX_POS.get() + c.len_utf8());

        if c == '\n' {
            LEX_LINENO.set(LEX_LINENO.get() + 1);
        }

        Some(c)
    }

    /// Put character back into input
    fn hungetc(&mut self, c: char) {
        LEX_UNGET_BUF.with_borrow_mut(|b| b.push_front(c));
        if c == '\n' && LEX_LINENO.get() > 1 {
            LEX_LINENO.set(LEX_LINENO.get() - 1);
        }
        LEX_LEXSTOP.set(false);
    }

    /// Peek at next character without consuming
    #[allow(dead_code)]
    fn peek(&mut self) -> Option<char> {
        if let Some(c) = LEX_UNGET_BUF.with_borrow(|b| b.front().copied()) {
            return Some(c);
        }
        LEX_INPUT.with_borrow(|s| s[LEX_POS.get()..].chars().next())
    }

    /// Add character to token buffer
    fn add(&mut self, c: char) {
        LEX_LEXBUF.with_borrow_mut(|b| b.add(c));
    }

    /// Check if character is blank (space or tab)
    fn is_blank(c: char) -> bool {
        c == ' ' || c == '\t'
    }

    /// Peek for a zsh numeric range glob shape after a `<`: returns the
    /// captured `N*-M*>` (everything *after* the leading `<`) when the
    /// upcoming chars match `[0-9]*-[0-9]*>` exactly. Otherwise returns
    /// None and leaves the input untouched.
    fn try_numeric_range_glob(&mut self) -> Option<String> {
        let mut buf: Vec<char> = Vec::new();
        // optional leading digits
        loop {
            match self.hgetc() {
                Some(c) if c.is_ascii_digit() => buf.push(c),
                Some(c) => {
                    buf.push(c);
                    break;
                }
                None => break,
            }
        }
        // last char in buf must be '-' for the range form
        if buf.last() != Some(&'-') {
            for c in buf.iter().rev() {
                self.hungetc(*c);
            }
            return None;
        }
        // optional trailing digits
        loop {
            match self.hgetc() {
                Some(c) if c.is_ascii_digit() => buf.push(c),
                Some(c) => {
                    buf.push(c);
                    break;
                }
                None => break,
            }
        }
        if buf.last() != Some(&'>') {
            for c in buf.iter().rev() {
                self.hungetc(*c);
            }
            return None;
        }
        Some(buf.into_iter().collect())
    }

    /// Check if character is blank (including other whitespace except newline)
    fn is_inblank(c: char) -> bool {
        matches!(c, ' ' | '\t' | '\x0b' | '\x0c' | '\r')
    }

    /// Check if character is a digit
    fn is_digit(c: char) -> bool {
        c.is_ascii_digit()
    }

    /// Check if character is identifier start
    #[allow(dead_code)]
    fn is_ident_start(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_'
    }

    /// Check if character is identifier continuation
    fn is_ident(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

    /// Main lexer entry point — fetch the next token. Direct port of
    /// zsh/Src/lex.c:265-313 `zshlex`. Loop body matches the C source
    /// `do { ... } while (tok != ENDINPUT && exalias())` at lex.c:270-276,
    /// followed by here-doc draining (lex.c:278-306), newline tracking
    /// (lex.c:307-310), and SEMI/NEWLIN→SEPER folding (lex.c:311-312).
    ///
    /// zshrs port note: `exalias()` (lex.c:1953) is not yet wired into
    /// the loop. The C source iterates as long as exalias keeps
    /// re-injecting alias text into the input buffer; zshrs's alias
    /// expansion happens post-lex in exec.rs. The loop body therefore
    /// runs once and breaks unconditionally — documented divergence.
    pub fn zshlex(&mut self) {
        // lex.c:268-269 — early-out on prior LEXERR.
        if self.tok() == LEXERR {
            return;
        }

        // Note: Do NOT reset global_iterations here - it must accumulate across all
        // zshlex calls in a parse to prevent infinite loops in the parser

        // lex.c:270-276 — gettok / exalias one-pass body. The C source
        // wraps gettok in `do { ... } while (exalias())` so an alias
        // re-injection re-enters the lex. Until exalias is wired we
        // run the body exactly once, no loop scaffolding.
        // lex.c:271-272 — bump inrepeat counter for `repeat N {}`
        // detection.
        if LEX_INREPEAT.get() > 0 {
            LEX_INREPEAT.set(LEX_INREPEAT.get() + 1);
        }
        // lex.c:273-274 — at the third token after `repeat`,
        // SHORTLOOPS / SHORTREPEAT options force back into cmd
        // position so the loop body can start. zshrs unconditionally
        // does this since the option-lookup lives in exec.rs.
        if LEX_INREPEAT.get() == 3 {
            LEX_INCMDPOS.set(true);
        }

        // lex.c:275 — `tok = gettok();`
        let _t = self.gettok();
        self.set_tok(_t);

        // lex.c:277 — `nocorrect &= 1;` — clear bit 1 (lookahead-only)
        // so the persistent low bit survives but the per-word bit is
        // dropped.
        LEX_NOCORRECT.set(LEX_NOCORRECT.get() & 1);

        // lex.c:278-306 — drain pending here-documents at the start
        // of a new line. zshrs's process_heredocs reads the full body
        // and stitches it onto the matching redir token.
        if self.tok() == NEWLIN || self.tok() == ENDINPUT {
            self.process_heredocs();
        }

        // lex.c:307-310 — track whether we just saw a newline.
        // C uses `inbufct` to distinguish "newline at EOF" (=1)
        // from "newline mid-input" (=-1); zshrs reads `pos < len`.
        if self.tok() != NEWLIN {
            LEX_ISNEWLIN.set(0);
        } else {
            LEX_ISNEWLIN.set(if LEX_POS.get() < LEX_INPUT.with_borrow(|s| s.len()) { -1 } else { 1 });
        }

        // lex.c:311-312 — fold SEMI / NEWLIN into SEPER unless
        // LEXFLAGS_NEWLINE is set to preserve newlines (used by
        // ZLE for completion of partial lines).
        if self.tok() == SEMI || (self.tok() == NEWLIN && LEX_LEXFLAGS.get() & LEXFLAGS_NEWLINE == 0) {
            self.set_tok(SEPER);
        }

        // Reserved-word promotion. Per lex.c:2002-2005 in `exalias`:
        //   - `{` only promotes to INBRACE in command position
        //   - `}` promotes to OUTBRACE either in cmdpos OR via the
        //     special `closing-brace-special` rule (IGNOREBRACES unset
        //     — assumed since zshrs doesn't expose that option yet)
        //   - other reserved words: only when incmdpos (or `}` exception)
        if self.tok() == STRING_LEX {
            let _t_s = self.tokstr();
            if let Some(s) = _t_s.as_deref() {
                if s == "{" && LEX_INCMDPOS.get() {
                    self.set_tok(INBRACE_TOK);
                } else if s == "}" {
                    self.set_tok(OUTBRACE_TOK);
                } else if LEX_INCASEPAT.get() == 0 {
                    // Skip reserved word checking in case pattern context —
                    // words like `time`, `end` should be patterns, not
                    // keywords.
                    self.check_reserved_word();
                }
            }
        }

        // If we were expecting a heredoc terminator, register it now
        if LEX_HEREDOC_PENDING.get() > 0 && self.tok() == STRING_LEX {
            let _t_terminator = self.tokstr();
            if let Some(terminator) = _t_terminator.as_deref() {
                let strip_tabs = LEX_HEREDOC_PENDING.get() == 2;
                // Detect originally-quoted terminator (`<<'EOF'`,
                // `<<"EOF"`). The lexer wraps single-quoted text in
                // SNULL (`\u{9d}`) and double-quoted text in DNULL
                // (`\u{9e}`); plain `EOF` has neither. Quoted-terminator
                // heredocs disable variable / command-sub / arithmetic
                // expansion in the body — see `compile_redir` for the
                // expansion side.
                // Quoted terminators (`<<'EOF'`, `<<"EOF"`, `<<\EOF`)
                // disable expansion in the body. SNULL/DNULL mark
                // single/double-quoted spans; BNULL (`\u{9f}`) marks
                // any backslash-escaped char — its presence alone is
                // enough to flag the terminator as quoted (zsh's
                // `<<\EOF` shorthand for `<<'EOF'`).
                let quoted = terminator.contains('\u{9d}')
                    || terminator.contains('\u{9e}')
                    || terminator.contains('\u{9f}')
                    || terminator.starts_with('\'')
                    || terminator.starts_with('"');
                let term = terminator
                    .chars()
                    .filter(|c| {
                        *c != '\''
                            && *c != '"'
                            && *c != '\u{9d}'
                            && *c != '\u{9e}'
                            && *c != '\u{9f}'
                    })
                    .collect::<String>();
                LEX_HEREDOCS.with_borrow_mut(|v| v.push(HereDoc {
                    terminator: term,
                    strip_tabs,
                    content: String::new(),
                    quoted,
                    processed: false,
                }));
            }
            LEX_HEREDOC_PENDING.set(0);
        }

        // Track pattern context inside [[ ... ]] - after = == != =~ the RHS is a pattern
        if LEX_INCOND.get() > 0 {
            let _t_s = self.tokstr();
            if let Some(s) = _t_s.as_deref() {
                // Check if this token is a comparison operator
                // Note: single = is also a comparison operator in [[ ]]
                // The internal marker \u{8d} is used for =
                if s == "="
                    || s == "=="
                    || s == "!="
                    || s == "=~"
                    || s == "\u{8d}"
                    || s == "\u{8d}\u{8d}"
                    || s == "!\u{8d}"
                    || s == "\u{8d}~"
                    || s == "\u{8d}\u{98}"
                {
                    LEX_INCONDPAT.set(true);
                } else if LEX_INCONDPAT.get() {
                    // We were in pattern context, now we've consumed the pattern
                    // Reset after the pattern token is consumed
                    // But actually, pattern can span multiple tokens, so we should
                    // stay in pattern mode until ]] or && or ||
                }
            }
            // Reset pattern context on ]] or logical operators (&&, ||)
            // and grouping parens. zsh par_cond_3 (cond.c) treats
            // these as cond-pattern terminators — the next operand is
            // a fresh primary, NOT a continuation of the prior pattern.
            // Without resetting on Damper/Dbar/Inpar/Outpar, the `(`
            // after `[[ a == a && (b == b ... ` was lexed as a literal
            // glob char (incondpat=true → gettokstr) and the whole
            // remainder collapsed into one String token.
            match self.tok() {
                DOUTBRACK
                | DAMPER
                | DBAR
                | INPAR_TOK
                | OUTPAR_TOK
                | BANG_TOK => {
                    LEX_INCONDPAT.set(false);
                }
                _ => {}
            }
        } else {
            LEX_INCONDPAT.set(false);
        }

        // Update command position for next token based on current token
        // Note: In case patterns (incasepat > 0), | is a pattern separator, not pipeline,
        // so we don't set incmdpos after Bar in that context
        match self.tok() {
            SEPER
            | NEWLIN
            | SEMI
            | DSEMI
            | SEMIAMP
            | SEMIBAR
            | AMPER
            | AMPERBANG
            | INPAR_TOK
            | INBRACE_TOK
            | DBAR
            | DAMPER
            | BARAMP
            | INOUTPAR
            | DOLOOP
            | THEN
            | ELIF
            | ELSE
            | DOUTBRACK
            | FUNC => {
                LEX_INCMDPOS.set(true);
            }
            BAR_TOK
                // In case patterns, | is a pattern separator - don't change incmdpos
                if LEX_INCASEPAT.get() <= 0 => {
                    LEX_INCMDPOS.set(true);
                }
            TYPESET => {
                LEX_INCMDPOS.set(false);
                // typeset / declare / local / export / readonly /
                // integer / float / autoload accept assignment-shape
                // args (NAME=value, NAME=()). Set intypeset so the
                // lexer's `=`-after-name detector emits Envstring/
                // Envarray for those args. Direct port of zsh's
                // lex.c which sets `intypeset` when one of the
                // typeset-family commands is seen at cmdpos.
                LEX_INTYPESET.set(true);
            }
            STRING_LEX
            | ENVARRAY
            | OUTPAR_TOK
            | CASE
            | DINBRACK => {
                LEX_INCMDPOS.set(false);
            }
            // Command separators clear the intypeset bit so the next
            // command's args don't get assignment-shape recognition.
            SEPER
            | NEWLIN
            | SEMI
            | DSEMI
            | SEMIAMP
            | SEMIBAR
            | AMPER
            | DAMPER
            | DBAR
            | BARAMP => {
                LEX_INTYPESET.set(false);
            }
            _ => {}
        }

        // Track 'for' keyword for C-style for loop: for (( init; cond; step ))
        // When we see 'for', set infor=2 to expect the init and cond parts
        // Each Dinpar (after semicolon in arithmetic) decrements it
        if self.tok() != DINPAR {
            LEX_INFOR.set(if self.tok() == FOR { 2 } else { 0 });
        }


        // Handle redirection / for-loop context. Mirrors lex.c:359-368
        // ctxtlex `oldpos` save/restore. The saved value lives in
        // `LEX_OLDPOS.get()` (struct field) so it survives across zshlex
        // calls — the previous local `let oldpos = LEX_INCMDPOS.get()`
        // captured the JUST-updated value (always wrong) and lost the
        // pre-FOR incmdpos. With the field, FOR x → STRING_TOK x → INPAR
        // sequence correctly restores incmdpos=1 before the `(`.
        if IS_REDIROP(self.tok())
            || self.tok() == FOR
            || self.tok() == FOREACH
            || self.tok() == SELECT
        {
            LEX_INREDIR.set(true);
            LEX_OLDPOS.set(LEX_INCMDPOS.get());
            LEX_INCMDPOS.set(false);
        } else if LEX_INREDIR.get() {
            LEX_INCMDPOS.set(LEX_OLDPOS.get());
            LEX_INREDIR.set(false);
        }
    }

    /// Process pending here-documents. Walks each heredoc whose body
    /// hasn't been filled yet (content is empty AND terminator is set),
    /// reads lines from input until the terminator, and stuffs the body
    /// into `hdoc.content` IN PLACE. The list itself is preserved so the
    /// parser can index into it after parse() finishes.
    fn process_heredocs(&mut self) {
        let n = LEX_HEREDOCS.with_borrow(|v| v.len());
        for i in 0..n {
            // Skip heredocs we've already processed AND those without
            // a terminator (early-error case). The `processed` bool
            // distinguishes "filled with empty body" from "not yet
            // visited" — both have empty `content`.
            let (skip, strip_tabs, terminator) = LEX_HEREDOCS.with_borrow(|v| {
                if v[i].processed || v[i].terminator.is_empty() {
                    (true, false, String::new())
                } else {
                    (false, v[i].strip_tabs, v[i].terminator.clone())
                }
            });
            if skip {
                continue;
            }
            let mut content = String::new();
            let mut line_count = 0;

            loop {
                line_count += 1;
                if line_count > 10000 {
                    LEX_ERROR.with_borrow_mut(|e| *e = Some("heredoc exceeded 10000 lines".to_string()));
                    self.set_tok(LEXERR);
                    return;
                }

                let line = self.read_line();
                if line.is_none() {
                    LEX_ERROR.with_borrow_mut(|e| *e = Some("here document too large or unterminated".to_string()));
                    self.set_tok(LEXERR);
                    return;
                }

                let line = line.unwrap();
                let check_line = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };

                if check_line.trim_end_matches('\n') == terminator {
                    break;
                }

                // `<<-` strips leading tabs from BODY lines too, not just
                // from terminator-match comparison. Without this, tabs in
                // here-doc content survive into stdin.
                if strip_tabs {
                    content.push_str(check_line);
                } else {
                    content.push_str(&line);
                }
            }

            LEX_HEREDOCS.with_borrow_mut(|v| {
                v[i].content = content;
                v[i].processed = true;
            });
        }
    }

    /// Read a line from input (returns partial line at EOF)
    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();

        loop {
            match self.hgetc() {
                Some(c) => {
                    line.push(c);
                    if c == '\n' {
                        break;
                    }
                }
                None => {
                    // EOF - return partial line if any
                    if line.is_empty() {
                        return None;
                    }
                    break;
                }
            }
        }

        Some(line)
    }

    /// Get the next token. Direct port of zsh/Src/lex.c:613-936
    /// `gettok`. Reads characters from the input via hgetc, dispatches
    /// on the leading char through lexact1[]/lexact2[] tables (zshrs
    /// uses inline `match` in lex_initial / lex_inang / lex_outang
    /// since Rust pattern-matching subsumes the table dispatch).
    ///
    /// Structural divergence from C: the giant ~322-line C switch
    /// statement at lex.c:725-936 is split into helper methods in
    /// Rust (lex_initial = LX1_OTHER plus the punctuation cases,
    /// lex_inang / lex_outang for the < and > arms). The flow is
    /// equivalent — same chars consumed, same tokens emitted — but
    /// the source-level layout differs. C's table-driven dispatch
    /// would Rust-port as `match c { '\\' => ..., '\n' => ..., ... }`
    /// which is what the helpers ultimately do.
    fn gettok(&mut self) -> lextok {
        // lex.c:621 — `tokstr = NULL;` reset before each token.
        self.set_tokstr(None);
        // (zshrs-specific: tokfd reset lives here too — C does it
        // implicitly via the `peekfd = -1` local at lex.c:617 used
        // only when a digit-prefix redirection is detected.)
        LEX_TOKFD.set(-1);

        // lex.c:622 — `while (iblank(c = hgetc()) && !lexstop);` —
        // skip leading blanks (space/tab, NOT newline).
        let mut ws_iterations = 0;
        loop {
            ws_iterations += 1;
            if ws_iterations > 100_000 {
                LEX_ERROR.with_borrow_mut(|e| *e = Some("gettok: infinite loop in whitespace skip".to_string()));
                return LEXERR;
            }
            let c = match self.hgetc() {
                Some(c) => c,
                None => {
                    // lex.c:624-625 — lexstop set, return ENDINPUT
                    // (or LEXERR if errflag is set elsewhere).
                    LEX_LEXSTOP.set(true);
                    return if LEX_ERROR.with_borrow(|e| e.is_some()) {
                        LEXERR
                    } else {
                        ENDINPUT
                    };
                }
            };

            if !Self::is_blank(c) {
                self.hungetc(c);
                break;
            }
        }

        let c = match self.hgetc() {
            Some(c) => c,
            None => {
                LEX_LEXSTOP.set(true);
                return ENDINPUT;
            }
        };

        // lex.c:623 — `toklineno = lineno;`
        LEX_TOKLINENO.set(LEX_LINENO.get());
        // lex.c:626 — `isfirstln = 0;` once we've consumed any non-
        // blank.
        LEX_ISFIRSTLN.set(false);

        // lex.c:631-648 — dbparens (inside `(( … ))`) special path:
        // call dquote_parse with `;` or `)` as the end-char and
        // either return DINPAR (continue for-loop arith) or DOUTPAR
        // (close the arith block) or LEXERR.
        if LEX_DBPARENS.get() {
            return self.lex_arith(c);
        }

        // lex.c:649-668 — digit prefix on a redirection: `2> file`
        // treats `2` as the fd to redirect, not a literal arg. Three
        // shapes: `N>`/`N<` (single redir), `N&>` (errwrite), or
        // anything else (push back, treat as literal digit).
        if Self::is_digit(c) {
            let d = self.hgetc();
            match d {
                Some('&') => {
                    let e = self.hgetc();
                    if e == Some('>') {
                        // lex.c:653-657 — `N&>` shape detected.
                        LEX_TOKFD.set((c as u8 - b'0') as i32);
                        self.hungetc('>');
                        return self.lex_initial('&');
                    }
                    // lex.c:658-661 — not `N&>`, push everything back.
                    if let Some(e) = e {
                        self.hungetc(e);
                    }
                    self.hungetc('&');
                }
                Some('>') | Some('<') => {
                    // lex.c:662-664 — `N>` or `N<` shape detected.
                    LEX_TOKFD.set((c as u8 - b'0') as i32);
                    return self.lex_initial(d.unwrap());
                }
                Some(d) => {
                    // lex.c:665-668 — not a redir prefix, push back.
                    self.hungetc(d);
                }
                None => {}
            }
            LEX_LEXSTOP.set(false);
        }

        // lex.c:670-936 — main dispatch on the leading char. zshrs
        // delegates to lex_initial which holds the equivalent of
        // lex.c's `switch (lexact1[c])` plus the gettokstr fallback
        // for LX1_OTHER.
        self.lex_initial(c)
    }

    /// Lex (( ... )) arithmetic expression
    fn lex_arith(&mut self, c: char) -> lextok {
        LEX_LEXBUF.with_borrow_mut(|b| b.clear());
        self.hungetc(c);

        let end_char = if LEX_INFOR.get() > 0 { ';' } else { ')' };
        if self.dquote_parse(end_char, false).is_err() {
            return LEXERR;
        }

        self.set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));

        if !LEX_LEXSTOP.get() && LEX_INFOR.get() > 0 {
            LEX_INFOR.set(LEX_INFOR.get() - 1);
            return DINPAR;
        }

        // Check for closing ))
        match self.hgetc() {
            Some(')') => {
                LEX_DBPARENS.set(false);
                DOUTPAR
            }
            c => {
                if let Some(c) = c {
                    self.hungetc(c);
                }
                LEXERR
            }
        }
    }

    /// Handle initial character of token
    fn lex_initial(&mut self, c: char) -> lextok {
        // Handle comments
        if c == '#' && !LEX_NOCOMMENTS.get() {
            return self.lex_comment();
        }

        match c {
            '\\' => {
                let d = self.hgetc();
                if d == Some('\n') {
                    // Line continuation - get next token
                    return self.gettok();
                }
                if let Some(d) = d {
                    self.hungetc(d);
                }
                LEX_LEXSTOP.set(false);
                self.gettokstr(c, false)
            }

            '\n' => NEWLIN,

            ';' => {
                let d = self.hgetc();
                match d {
                    Some(';') => DSEMI,
                    Some('&') => SEMIAMP,
                    Some('|') => SEMIBAR,
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        LEX_LEXSTOP.set(false);
                        SEMI
                    }
                }
            }

            '&' => {
                let d = self.hgetc();
                match d {
                    Some('&') => DAMPER,
                    Some('!') | Some('|') => AMPERBANG,
                    Some('>') => {
                        LEX_TOKFD.set(LEX_TOKFD.get().max(0));
                        let e = self.hgetc();
                        match e {
                            Some('!') | Some('|') => OUTANGAMPBANG,
                            Some('>') => {
                                let f = self.hgetc();
                                match f {
                                    Some('!') | Some('|') => DOUTANGAMPBANG,
                                    _ => {
                                        if let Some(f) = f {
                                            self.hungetc(f);
                                        }
                                        LEX_LEXSTOP.set(false);
                                        DOUTANGAMP
                                    }
                                }
                            }
                            _ => {
                                if let Some(e) = e {
                                    self.hungetc(e);
                                }
                                LEX_LEXSTOP.set(false);
                                AMPOUTANG
                            }
                        }
                    }
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        LEX_LEXSTOP.set(false);
                        AMPER
                    }
                }
            }

            '|' => {
                let d = self.hgetc();
                match d {
                    Some('|') if LEX_INCASEPAT.get() <= 0 => DBAR,
                    Some('&') => BARAMP,
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        LEX_LEXSTOP.set(false);
                        BAR_TOK
                    }
                }
            }

            '(' => {
                let d = self.hgetc();
                match d {
                    Some('(') => {
                        if LEX_INFOR.get() > 0 {
                            LEX_DBPARENS.set(true);
                            return DINPAR;
                        }
                        if LEX_INCMDPOS.get() {
                            // Could be (( arithmetic )) or ( subshell )
                            LEX_LEXBUF.with_borrow_mut(|b| b.clear());
                            match self.cmd_or_math() {
                                CMD_OR_MATH_MATH => {
                                    self.set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));
                                    return DINPAR;
                                }
                                CMD_OR_MATH_CMD => {
                                    self.set_tokstr(None);
                                    return INPAR_TOK;
                                }
                                CMD_OR_MATH_ERR | _ => return LEXERR,
                            }
                        }
                        self.hungetc('(');
                        LEX_LEXSTOP.set(false);
                        self.gettokstr('(', false)
                    }
                    Some(')') => INOUTPAR,
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        LEX_LEXSTOP.set(false);
                        // Per lex.c:822 LX1_INPAR — at word boundary `(`
                        // tokenizes as INPAR when SHGLOB || incond==1 ||
                        // incmdpos. Otherwise falls through to gettokstr
                        // (the `(` becomes start of a STRING_TOK — typical
                        // for unquoted glob args like `ls (^foo)*`).
                        // For `for x ( ... )` form, incmdpos is restored
                        // to 1 via the oldpos-save-after-FOR mechanism,
                        // so the next-token `(` correctly INPAR-izes.
                        if LEX_INCOND.get() == 1 || LEX_INCMDPOS.get() || LEX_INCASEPAT.get() >= 1 {
                            INPAR_TOK
                        } else {
                            self.gettokstr('(', false)
                        }
                    }
                }
            }

            ')' => OUTPAR_TOK,

            '{' => {
                // { is a command group only if followed by whitespace,
                // newline, or `}` (the empty-block form `{}`). zsh
                // treats `{}` as an empty compound — `foo() {}` is a
                // valid no-op function. Without `}` in this list,
                // `{}` got consumed as one literal token and ran as a
                // command, failing "command not found: {}".
                // The empty `{}` is also recognised AFTER a function
                // header `name()` even when `incmdpos` got cleared by
                // the preceding Outpar — peek for `}` regardless and
                // treat as Inbrace so `foo() {}` parses as a no-op
                // function body.
                let next = self.hgetc();
                let next_is_close = matches!(next, Some('}'));
                if LEX_INCMDPOS.get() {
                    let is_brace_group = matches!(next, Some(' ' | '\t' | '\n' | '}') | None);
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    if is_brace_group {
                        self.set_tokstr(Some("{".to_string()));
                        INBRACE_TOK
                    } else {
                        self.gettokstr(c, false)
                    }
                } else if next_is_close {
                    // `{}` empty block in non-cmd position (function
                    // body after `()`). Treat as Inbrace; the parser
                    // will follow with Outbrace.
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    self.set_tokstr(Some("{".to_string()));
                    INBRACE_TOK
                } else {
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    self.gettokstr(c, false)
                }
            }

            '}' => {
                // } at start of token is always Outbrace (ends command group)
                // Inside a word, } would be handled by gettokstr but we never reach here mid-word
                self.set_tokstr(Some("}".to_string()));
                OUTBRACE_TOK
            }

            '[' => {
                // [[ is a conditional expression start
                // [ can also be a command (test builtin) or array subscript
                // In case patterns (incasepat > 0), [ is part of glob pattern like [yY]
                if LEX_INCASEPAT.get() > 0 {
                    self.gettokstr(c, false)
                } else if LEX_INCMDPOS.get() {
                    let next = self.hgetc();
                    if next == Some('[') {
                        // [[ - double bracket conditional
                        self.set_tokstr(Some("[[".to_string()));
                        LEX_INCOND.set(1);
                        return DINBRACK;
                    }
                    // Single [ - either test command or start of glob pattern
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    self.set_tokstr(Some("[".to_string()));
                    STRING_LEX
                } else {
                    self.gettokstr(c, false)
                }
            }

            ']' => {
                // ]] ends a conditional expression started by [[
                if LEX_INCOND.get() > 0 {
                    let next = self.hgetc();
                    if next == Some(']') {
                        self.set_tokstr(Some("]]".to_string()));
                        LEX_INCOND.set(0);
                        return DOUTBRACK;
                    }
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                }
                self.gettokstr(c, false)
            }

            '<' => {
                // In pattern context, < is literal (e.g., <-> in glob)
                if LEX_INCONDPAT.get() || LEX_INCASEPAT.get() > 0 {
                    self.gettokstr(c, false)
                } else {
                    self.lex_inang()
                }
            }

            '>' => {
                // In pattern context, > is literal
                if LEX_INCONDPAT.get() || LEX_INCASEPAT.get() > 0 {
                    self.gettokstr(c, false)
                } else {
                    self.lex_outang()
                }
            }

            _ => self.gettokstr(c, false),
        }
    }

    /// Lex comment
    fn lex_comment(&mut self) -> lextok {
        if LEX_LEXFLAGS.get() & LEXFLAGS_COMMENTS_KEEP != 0 {
            LEX_LEXBUF.with_borrow_mut(|b| b.clear());
            self.add('#');
        }

        loop {
            let c = self.hgetc();
            match c {
                Some('\n') | None => break,
                Some(c) => {
                    if LEX_LEXFLAGS.get() & LEXFLAGS_COMMENTS_KEEP != 0 {
                        self.add(c);
                    }
                }
            }
        }

        if LEX_LEXFLAGS.get() & LEXFLAGS_COMMENTS_KEEP != 0 {
            self.set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));
            if !LEX_LEXSTOP.get() {
                self.hungetc('\n');
            }
            return STRING_LEX;
        }

        if LEX_LEXFLAGS.get() & LEXFLAGS_COMMENTS_STRIP != 0 && LEX_LEXSTOP.get() {
            return ENDINPUT;
        }

        NEWLIN
    }

    /// Lex < and variants
    fn lex_inang(&mut self) -> lextok {
        let d = self.hgetc();
        match d {
            Some('(') => {
                // Process substitution <(...)
                self.hungetc('(');
                LEX_LEXSTOP.set(false);
                self.gettokstr('<', false)
            }
            Some('>') => INOUTANG,
            Some('<') => {
                let e = self.hgetc();
                match e {
                    Some('(') => {
                        self.hungetc('(');
                        self.hungetc('<');
                        INANG_TOK
                    }
                    Some('<') => TRINANG,
                    Some('-') => {
                        LEX_HEREDOC_PENDING.set(2); // <<- expects terminator next
                        DINANGDASH
                    }
                    _ => {
                        if let Some(e) = e {
                            self.hungetc(e);
                        }
                        LEX_LEXSTOP.set(false);
                        LEX_HEREDOC_PENDING.set(1); // << expects terminator next
                        DINANG
                    }
                }
            }
            Some('&') => INANGAMP,
            _ => {
                if let Some(d) = d {
                    self.hungetc(d);
                }
                LEX_LEXSTOP.set(false);
                INANG_TOK
            }
        }
    }

    /// Lex > and variants
    fn lex_outang(&mut self) -> lextok {
        let d = self.hgetc();
        match d {
            Some('(') => {
                // Process substitution >(...)
                self.hungetc('(');
                LEX_LEXSTOP.set(false);
                self.gettokstr('>', false)
            }
            Some('&') => {
                let e = self.hgetc();
                match e {
                    Some('!') | Some('|') => OUTANGAMPBANG,
                    _ => {
                        if let Some(e) = e {
                            self.hungetc(e);
                        }
                        LEX_LEXSTOP.set(false);
                        OUTANGAMP
                    }
                }
            }
            Some('!') | Some('|') => OUTANGBANG,
            Some('>') => {
                let e = self.hgetc();
                match e {
                    Some('&') => {
                        let f = self.hgetc();
                        match f {
                            Some('!') | Some('|') => DOUTANGAMPBANG,
                            _ => {
                                if let Some(f) = f {
                                    self.hungetc(f);
                                }
                                LEX_LEXSTOP.set(false);
                                DOUTANGAMP
                            }
                        }
                    }
                    Some('!') | Some('|') => DOUTANGBANG,
                    Some('(') => {
                        self.hungetc('(');
                        self.hungetc('>');
                        OUTANG_TOK
                    }
                    _ => {
                        if let Some(e) = e {
                            self.hungetc(e);
                        }
                        LEX_LEXSTOP.set(false);
                        DOUTANG
                    }
                }
            }
            _ => {
                if let Some(d) = d {
                    self.hungetc(d);
                }
                LEX_LEXSTOP.set(false);
                OUTANG_TOK
            }
        }
    }

    /// Get rest of token string
    fn gettokstr(&mut self, c: char, sub: bool) -> lextok {
        let mut bct = 0; // brace count
        let mut pct = 0; // parenthesis count
        let mut brct = 0; // bracket count
        let mut in_brace_param = 0;
        let mut peek = STRING_LEX;
        let mut intpos = 1;
        let mut unmatched = '\0';
        let mut c = c;
        const MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0;

        if !sub {
            LEX_LEXBUF.with_borrow_mut(|b| b.clear());
        }

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                LEX_ERROR.with_borrow_mut(|e| *e = Some("gettokstr exceeded maximum iterations".to_string()));
                return LEXERR;
            }

            let inbl = Self::is_inblank(c);

            if inbl && in_brace_param == 0 && pct == 0 {
                // Whitespace outside brace param ends token
                break;
            }

            match c {
                // Whitespace is handled above for most cases
                ')' => {
                    if in_brace_param > 0 || sub {
                        self.add(OUTPAR);
                    } else if pct > 0 {
                        pct -= 1;
                        self.add(OUTPAR);
                    } else {
                        break;
                    }
                }

                '|' => {
                    if pct == 0 && in_brace_param == 0 {
                        if sub {
                            self.add(c);
                        } else {
                            break;
                        }
                    } else {
                        self.add(BAR);
                    }
                }

                '$' => {
                    let e = self.hgetc();
                    match e {
                        Some('\\') => {
                            let f = self.hgetc();
                            if f != Some('\n') {
                                if let Some(f) = f {
                                    self.hungetc(f);
                                }
                                self.hungetc('\\');
                                self.add(STRING_TOK);
                            } else {
                                // Line continuation after $
                                continue;
                            }
                        }
                        Some('[') => {
                            // $[...] arithmetic
                            self.add(STRING_TOK);
                            self.add(INBRACK);
                            if self.dquote_parse(']', sub).is_err() {
                                peek = LEXERR;
                                break;
                            }
                            self.add(OUTBRACK);
                        }
                        Some('(') => {
                            // $(...) or $((...))
                            self.add(STRING_TOK);
                            match self.cmd_or_math_sub() {
                                CMD_OR_MATH_CMD => self.add(OUTPAR),
                                CMD_OR_MATH_MATH => self.add(OUTPARMATH),
                                CMD_OR_MATH_ERR | _ => {
                                    peek = LEXERR;
                                    break;
                                }
                            }
                        }
                        Some('{') => {
                            self.add(c);
                            self.add(INBRACE);
                            bct += 1;
                            if in_brace_param == 0 {
                                in_brace_param = bct;
                            }
                        }
                        Some('\'') => {
                            // $'...' ANSI-C escape syntax.
                            // Port of Src/lex.c:1284-1314 (LX2_QUOTE
                            // branch when prev char was `String`):
                            // only `\\` and `\'` emit a `Bnull`
                            // marker (so getkeystring later
                            // recognizes them as user-literal); any
                            // other `\X` emits a literal `\` + the
                            // following char so getkeystring's
                            // standard `\n`/`\x`/`\u`/... decoding
                            // can fire.
                            self.add(QSTRING);
                            self.add(SNULL);
                            loop {
                                let ch = self.hgetc();
                                match ch {
                                    Some('\'') => break,
                                    Some('\\') => {
                                        let next = self.hgetc();
                                        match next {
                                            Some(n) => {
                                                if n == '\\' || n == '\'' {
                                                    self.add(BNULL);
                                                } else {
                                                    self.add('\\');
                                                }
                                                self.add(n);
                                            }
                                            None => {
                                                LEX_LEXSTOP.set(true);
                                                unmatched = '\'';
                                                peek = LEXERR;
                                                break;
                                            }
                                        }
                                    }
                                    Some(ch) => self.add(ch),
                                    None => {
                                        LEX_LEXSTOP.set(true);
                                        unmatched = '\'';
                                        peek = LEXERR;
                                        break;
                                    }
                                }
                            }
                            if unmatched != '\0' {
                                break;
                            }
                            self.add(SNULL);
                        }
                        Some('"') => {
                            // $"..." localized string. Same shape as a
                            // plain "..." but flagged via QSTRING+DNULL
                            // so post-lex translation can substitute.
                            self.add(QSTRING);
                            self.add(DNULL);
                            if self.dquote_parse('"', sub).is_err() {
                                peek = LEXERR;
                                break;
                            }
                            self.add(DNULL);
                        }
                        _ => {
                            if let Some(e) = e {
                                self.hungetc(e);
                            }
                            LEX_LEXSTOP.set(false);
                            self.add(STRING_TOK);
                        }
                    }
                }

                '[' => {
                    if in_brace_param == 0 {
                        brct += 1;
                    }
                    self.add(INBRACK);
                }

                ']' => {
                    if in_brace_param == 0 && brct > 0 {
                        brct -= 1;
                    }
                    self.add(OUTBRACK);
                }

                '(' => {
                    // lex.c:1078-1135 LX2_INPAR — when `(` appears inside
                    // a STRING_TOK and is immediately followed by `)`, the
                    // string terminates at the `(`. The `()` is then
                    // re-lexed as a separate INOUTPAR token. This handles
                    // function definitions: `name()` lexes as STRING_TOK `name`
                    // + INOUTPAR `()`, not STRING_TOK `name()`.
                    //
                    // Also (lex.c:1109-1112): under SHGLOB, a `(` followed
                    // by whitespace at the start of a command-position word
                    // (no nested brackets/braces) is a ksh function
                    // definition signal — same break-out behavior.
                    if in_brace_param == 0 && !sub {
                        let e = self.hgetc();
                        if let Some(ch) = e {
                            self.hungetc(ch);
                        }
                        LEX_LEXSTOP.set(false);
                        if e == Some(')') {
                            // `name()` — terminate STRING_TOK at `(` so the
                            // following `()` re-lexes as INOUTPAR. The
                            // loop's exit guard at line 2067 will
                            // `hungetc(c)` to push the `(` back; we only
                            // need to ensure `)` is also there. The
                            // hungetc(ch) above already pushed `)`, so
                            // breaking here yields unget_buf = [`(`, `)`]
                            // after the guard, which the outer dispatch
                            // reads as Inoutpar.
                            break;
                        }
                    }
                    if in_brace_param == 0 {
                        pct += 1;
                    }
                    self.add(INPAR);
                }

                '{' => {
                    // Track braces for both ${...} param expansion and {...} brace expansion
                    bct += 1;
                    self.add(c);
                }

                '}' => {
                    if in_brace_param > 0 {
                        if bct == in_brace_param {
                            in_brace_param = 0;
                        }
                        bct -= 1;
                        self.add(OUTBRACE);
                    } else if bct > 0 {
                        // Closing a brace expansion like {a,b}
                        bct -= 1;
                        self.add(c);
                    } else {
                        break;
                    }
                }

                '>' => {
                    // In pattern context (incondpat), > is literal
                    if in_brace_param > 0 || sub || LEX_INCONDPAT.get() || LEX_INCASEPAT.get() > 0 {
                        self.add(c);
                    } else {
                        let e = self.hgetc();
                        if e != Some('(') {
                            if let Some(e) = e {
                                self.hungetc(e);
                            }
                            LEX_LEXSTOP.set(false);
                            break;
                        }
                        // >(...)
                        self.add(OUTANG_PROC);
                        if self.skip_command_sub().is_err() {
                            peek = LEXERR;
                            break;
                        }
                        self.add(OUTPAR);
                    }
                }

                '<' => {
                    // In pattern context (incondpat), < is literal
                    if in_brace_param > 0 || sub || LEX_INCONDPAT.get() || LEX_INCASEPAT.get() > 0 {
                        self.add(c);
                    } else if let Some(range_chars) = self.try_numeric_range_glob() {
                        // zsh numeric range glob `<N-M>`, `<->`, `<N->`,
                        // `<-M>`. When `<` mid-word matches that exact
                        // shape, swallow it into the word instead of
                        // breaking out for redirection.
                        self.add(c);
                        for ch in range_chars.chars() {
                            self.add(ch);
                        }
                    } else {
                        let e = self.hgetc();
                        if e != Some('(') {
                            if let Some(e) = e {
                                self.hungetc(e);
                            }
                            LEX_LEXSTOP.set(false);
                            break;
                        }
                        // <(...)
                        self.add(INANG);
                        if self.skip_command_sub().is_err() {
                            peek = LEXERR;
                            break;
                        }
                        self.add(OUTPAR);
                    }
                }

                '=' => {
                    if !sub {
                        if intpos > 0 {
                            // At start of token, check for =(...) process substitution
                            let e = self.hgetc();
                            if e == Some('(') {
                                self.add(EQUALS);
                                if self.skip_command_sub().is_err() {
                                    peek = LEXERR;
                                    break;
                                }
                                self.add(OUTPAR);
                            } else {
                                if let Some(e) = e {
                                    self.hungetc(e);
                                }
                                LEX_LEXSTOP.set(false);
                                self.add(EQUALS);
                            }
                        } else if peek != ENVSTRING
                            && (LEX_INCMDPOS.get() || LEX_INTYPESET.get())
                            && bct == 0
                            && brct == 0
                            && LEX_INCASEPAT.get() == 0
                        {
                            // Check for VAR=value assignment (but not in case pattern context)
                            let tok_so_far = LEX_LEXBUF.with_borrow(|b| b.as_str().to_string());
                            if self.is_valid_assignment_target(&tok_so_far) {
                                let next = self.hgetc();
                                if next == Some('(') {
                                    // VAR=(...) array assignment. Per zsh
                                    // (lex.c emits ENVARRAY with tokstr =
                                    // just the variable name, NOT
                                    // including the `=`). The `=` and
                                    // `(` are consumed by the lexer; the
                                    // parser knows ENVARRAY means assign-
                                    // array and reads the body that
                                    // follows.
                                    self.set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));
                                    return ENVARRAY;
                                }
                                if let Some(next) = next {
                                    self.hungetc(next);
                                }
                                LEX_LEXSTOP.set(false);
                                peek = ENVSTRING;
                                intpos = 2;
                                self.add(EQUALS);
                            } else {
                                self.add(EQUALS);
                            }
                        } else {
                            self.add(EQUALS);
                        }
                    } else {
                        self.add(EQUALS);
                    }
                }

                '\\' => {
                    let next = self.hgetc();
                    if next == Some('\n') {
                        // Line continuation
                        let next = self.hgetc();
                        if let Some(next) = next {
                            c = next;
                            continue;
                        }
                        break;
                    } else {
                        self.add(BNULL);
                        if let Some(next) = next {
                            self.add(next);
                        }
                    }
                }

                '\'' => {
                    // Single quoted string - everything literal until '
                    self.add(SNULL);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('\'') => break,
                            Some(ch) => self.add(ch),
                            None => {
                                LEX_LEXSTOP.set(true);
                                unmatched = '\'';
                                peek = LEXERR;
                                break;
                            }
                        }
                    }
                    if unmatched != '\0' {
                        break;
                    }
                    self.add(SNULL);
                }

                '"' => {
                    // Double quoted string
                    self.add(DNULL);
                    if self.dquote_parse('"', sub).is_err() {
                        unmatched = '"';
                        if LEX_LEXFLAGS.get() & LEXFLAGS_ACTIVE == 0 {
                            peek = LEXERR;
                        }
                        break;
                    }
                    self.add(DNULL);
                }

                '`' => {
                    // Backtick command substitution
                    self.add(TICK);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('`') => break,
                            Some('\\') => {
                                let next = self.hgetc();
                                match next {
                                    Some('\n') => continue, // Line continuation
                                    Some(c) if c == '`' || c == '\\' || c == '$' => {
                                        self.add(BNULL);
                                        self.add(c);
                                    }
                                    Some(c) => {
                                        self.add('\\');
                                        self.add(c);
                                    }
                                    None => break,
                                }
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                LEX_LEXSTOP.set(true);
                                unmatched = '`';
                                peek = LEXERR;
                                break;
                            }
                        }
                    }
                    if unmatched != '\0' {
                        break;
                    }
                    self.add(TICK);
                }

                '~' => {
                    self.add(TILDE);
                }

                '#' => {
                    self.add(POUND);
                }

                '^' => {
                    self.add(HAT);
                }

                '*' => {
                    self.add(STAR);
                }

                '?' => {
                    self.add(QUEST);
                }

                ',' if bct > in_brace_param => {
                    self.add(COMMA);
                }

                '-' => {
                    self.add(DASH);
                }

                '!' if brct > 0 => {
                    self.add(BANG);
                }

                // Terminators — but only when we're at the top level of
                // the current word. Inside a brace parameter expansion
                // `${...}`, parenthesized flag block `(@s.;.)`, or
                // bracketed subscript `[...]`, `;` is just a delimiter
                // character (e.g. the field separator in `(@s.;.)`),
                // not a statement terminator. Real zsh handles this
                // via gettokstr's incmdpos / bct / pct accounting; we
                // gate on the same counters.
                '\n' | ';' | '&' if in_brace_param == 0 && pct == 0 && brct == 0 => {
                    break;
                }
                '\n' | ';' | '&' => {
                    self.add(c);
                }

                _ => {
                    self.add(c);
                }
            }

            c = match self.hgetc() {
                Some(c) => c,
                None => {
                    LEX_LEXSTOP.set(true);
                    break;
                }
            };

            if intpos > 0 {
                intpos -= 1;
            }
        }

        // Put back the character that ended the token
        if !LEX_LEXSTOP.get() {
            self.hungetc(c);
        }

        if unmatched != '\0' && LEX_LEXFLAGS.get() & LEXFLAGS_ACTIVE == 0 {
            LEX_ERROR.with_borrow_mut(|e| *e = Some(format!("unmatched {}", unmatched)));
        }

        if in_brace_param > 0 {
            LEX_ERROR.with_borrow_mut(|e| *e = Some("closing brace expected".to_string()));
        }

        self.set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));
        peek
    }

    /// Check if a string is a valid assignment target (identifier or array ref).
    ///
    /// zsh accepts identifier (`[A-Za-z_][A-Za-z0-9_]*`) optionally followed by
    /// a `[...]` subscript. Bare digits are NOT a valid lvalue (rejected at
    /// `if c.is_ascii_digit()` below — array index expressions like `arr[2]`
    /// are caught by the subscript handler, not here). And the first char
    /// must NOT be a zsh internal token byte — `$=foo` (where `$` becomes
    /// the STRING_TOK token 0x85) is parameter substitution with the `=` flag,
    /// NOT an envstring assignment.
    fn is_valid_assignment_target(&self, s: &str) -> bool {
        let mut chars = s.chars().peekable();

        // Reject leading token byte — `$VAR=` is parameter substitution,
        // not assignment. Same for `*=`, `?=`, etc.
        if let Some(&c) = chars.peek() {
            if itok(c as u8) {
                return false;
            }
        }

        // Check for leading digit (invalid)
        if let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                // Could be array index, check rest
                while let Some(&c) = chars.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    chars.next();
                }
                return chars.peek().is_none();
            }
        }

        // Check identifier
        let mut has_ident = false;
        while let Some(&c) = chars.peek() {
            if c == INBRACK || c == '[' {
                break;
            }
            if c == '+' {
                // foo+=value
                chars.next();
                return chars.peek().is_none() || chars.peek() == Some(&'=');
            }
            if !Self::is_ident(c) && c != STRING_TOK && !itok(c as u8) {
                return false;
            }
            has_ident = true;
            chars.next();
        }

        has_ident
    }

    /// Parse the body of a double-quoted string (or any context that
    /// uses double-quote tokenization — `(( ))`, `${...}`, `$( ( ) )`).
    /// Direct port of zsh/Src/lex.c:1486-1693 `dquote_parse`. Reads
    /// chars until `endchar` is seen at depth 0, handling escapes,
    /// `${...}` parameter substitutions, `$(...)` and backtick command
    /// substitutions, `$((...))` arithmetic, and inner double-quoted
    /// strings. The `sub` flag toggles substitution-context tokens
    /// (lex.c:1487 `int sub` argument).
    ///
    /// zshrs port note: the recursion guard at the top is a Rust
    /// safety net; the C source relies on the runtime stack. Inner
    /// logic delegates to `dquote_parse_inner` which holds the actual
    /// per-char state machine matching lex.c:1495-1692.
    fn dquote_parse(&mut self, endchar: char, sub: bool) -> Result<(), ()> {
        LEX_RECURSION_DEPTH.set(LEX_RECURSION_DEPTH.get() + 1);
        if self.check_recursion() {
            LEX_RECURSION_DEPTH.set(LEX_RECURSION_DEPTH.get() - 1);
            return Err(());
        }

        let result = self.dquote_parse_inner(endchar, sub);
        LEX_RECURSION_DEPTH.set(LEX_RECURSION_DEPTH.get() - 1);
        result
    }

    fn dquote_parse_inner(&mut self, endchar: char, sub: bool) -> Result<(), ()> {
        let mut pct = 0; // parenthesis count
        let mut brct = 0; // bracket count
        let mut bct = 0; // brace count (for ${...})
        let mut intick = false; // inside backtick
        let is_math = endchar == ')' || endchar == ']' || LEX_INFOR.get() > 0;
        const MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0;

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                LEX_ERROR.with_borrow_mut(|e| *e = Some("dquote_parse exceeded maximum iterations".to_string()));
                return Err(());
            }
            let c = self.hgetc();
            let c = match c {
                Some(c) if c == endchar && !intick && bct == 0 => {
                    if is_math && (pct > 0 || brct > 0) {
                        self.add(c);
                        if c == ')' {
                            pct -= 1;
                        } else if c == ']' {
                            brct -= 1;
                        }
                        continue;
                    }
                    return Ok(());
                }
                Some(c) => c,
                None => {
                    LEX_LEXSTOP.set(true);
                    return Err(());
                }
            };

            match c {
                '\\' => {
                    let next = self.hgetc();
                    match next {
                        Some('\n') if !sub => continue, // Line continuation
                        Some(c)
                            if c == '$'
                                || c == '\\'
                                || (c == '}' && !intick && bct > 0)
                                || c == endchar
                                || c == '`'
                                || (endchar == ']'
                                    && (c == '['
                                        || c == ']'
                                        || c == '('
                                        || c == ')'
                                        || c == '{'
                                        || c == '}'
                                        || (c == '"' && sub))) =>
                        {
                            self.add(BNULL);
                            self.add(c);
                        }
                        Some(c) => {
                            self.add('\\');
                            self.hungetc(c);
                            continue;
                        }
                        None => {
                            self.add('\\');
                        }
                    }
                }

                '$' => {
                    if intick {
                        self.add(c);
                        continue;
                    }
                    let next = self.hgetc();
                    match next {
                        Some('(') => {
                            self.add(QSTRING);
                            match self.cmd_or_math_sub() {
                                CMD_OR_MATH_CMD => self.add(OUTPAR),
                                CMD_OR_MATH_MATH => self.add(OUTPARMATH),
                                CMD_OR_MATH_ERR | _ => return Err(()),
                            }
                        }
                        Some('[') => {
                            self.add(STRING_TOK);
                            self.add(INBRACK);
                            self.dquote_parse(']', sub)?;
                            self.add(OUTBRACK);
                        }
                        Some('{') => {
                            self.add(QSTRING);
                            self.add(INBRACE);
                            bct += 1;
                        }
                        Some('$') => {
                            self.add(QSTRING);
                            self.add('$');
                        }
                        _ => {
                            if let Some(next) = next {
                                self.hungetc(next);
                            }
                            LEX_LEXSTOP.set(false);
                            self.add(QSTRING);
                        }
                    }
                }

                '}' => {
                    if intick || bct == 0 {
                        self.add(c);
                    } else {
                        self.add(OUTBRACE);
                        bct -= 1;
                    }
                }

                '`' => {
                    self.add(QTICK);
                    intick = !intick;
                }

                '(' => {
                    if !is_math || bct == 0 {
                        pct += 1;
                    }
                    self.add(c);
                }

                ')' => {
                    if !is_math || bct == 0 {
                        if pct == 0 && is_math {
                            return Err(());
                        }
                        pct -= 1;
                    }
                    self.add(c);
                }

                '[' => {
                    if !is_math || bct == 0 {
                        brct += 1;
                    }
                    self.add(c);
                }

                ']' => {
                    if !is_math || bct == 0 {
                        if brct == 0 && is_math {
                            return Err(());
                        }
                        brct -= 1;
                    }
                    self.add(c);
                }

                '"' => {
                    if intick || (endchar != '"' && bct == 0) {
                        self.add(c);
                    } else if bct > 0 {
                        self.add(DNULL);
                        self.dquote_parse('"', sub)?;
                        self.add(DNULL);
                    } else {
                        return Err(());
                    }
                }

                _ => {
                    self.add(c);
                }
            }
        }
    }

    /// Determine if (( is arithmetic or command
    /// Decide whether `( ... )` after a `$` is a math expression
    /// `$((...))` or a command substitution `$(...)`. Direct port of
    /// zsh/Src/lex.c:495-532 `cmd_or_math`. Tries dquote_parse first;
    /// if it succeeds AND the next char is `)` (closing the second
    /// paren of `(( ))`), it's math. Otherwise rewinds and treats as
    /// a command substitution.
    fn cmd_or_math(&mut self) -> i32 {
        let oldlen = LEX_LEXBUF.with_borrow(|b| b.buf_len());

        // Per lex.c:498-518 — `cmd_or_math` calls `dquote_parse(')')`
        // which fills lexbuf with ONLY the inner expression, then checks
        // for the closing `)`. The surrounding `((` / `))` are NOT added
        // to lexbuf. zshrs previously added INPAR + '(' before dquote and
        // ')' after, polluting DINPAR's tokstr with the literal parens.
        // Removed to match C exactly.
        if self.dquote_parse(')', false).is_err() {
            // Back up and try as command
            while LEX_LEXBUF.with_borrow(|b| b.buf_len()) > oldlen {
                if let Some(c) = LEX_LEXBUF.with_borrow_mut(|b| b.pop()) {
                    self.hungetc(c);
                }
            }
            self.hungetc('(');
            LEX_LEXSTOP.set(false);
            return if self.skip_command_sub().is_err() {
                CMD_OR_MATH_ERR
            } else {
                CMD_OR_MATH_CMD
            };
        }

        // Check for closing ) — matches C lex.c:511-512: success-with-`)`
        // means `((..))` was math. Don't add `)` to lexbuf.
        let c = self.hgetc();
        if c == Some(')') {
            return CMD_OR_MATH_MATH;
        }

        // Not math, back up
        if let Some(c) = c {
            self.hungetc(c);
        }
        LEX_LEXSTOP.set(false);

        // Back up token
        while LEX_LEXBUF.with_borrow(|b| b.buf_len()) > oldlen {
            if let Some(c) = LEX_LEXBUF.with_borrow_mut(|b| b.pop()) {
                self.hungetc(c);
            }
        }
        self.hungetc('(');

        if self.skip_command_sub().is_err() {
            CMD_OR_MATH_ERR
        } else {
            CMD_OR_MATH_CMD
        }
    }

    /// Parse `$(...)` or `$((...))` after the `$` has been consumed.
    /// Direct port of zsh/Src/lex.c:540-573 `cmd_or_math_sub`. Reads
    /// the next char to discriminate: a leading `(` plus successful
    /// math parse via `cmd_or_math` → arithmetic substitution (with
    /// the open-paren retroactively rewritten to Inparmath); else
    /// command substitution via skip_command_sub.
    fn cmd_or_math_sub(&mut self) -> i32 {
        const MAX_CONTINUATIONS: usize = 10_000;
        let mut continuations = 0;

        loop {
            continuations += 1;
            if continuations > MAX_CONTINUATIONS {
                LEX_ERROR.with_borrow_mut(|e| *e = Some("cmd_or_math_sub: too many line continuations".to_string()));
                return CMD_OR_MATH_ERR;
            }

            let c = self.hgetc();
            if c == Some('\\') {
                let c2 = self.hgetc();
                if c2 != Some('\n') {
                    if let Some(c2) = c2 {
                        self.hungetc(c2);
                    }
                    self.hungetc('\\');
                    LEX_LEXSTOP.set(false);
                    return if self.skip_command_sub().is_err() {
                        CMD_OR_MATH_ERR
                    } else {
                        CMD_OR_MATH_CMD
                    };
                }
                // Line continuation, try again (loop instead of recursion)
                continue;
            }

            // Not a line continuation, process normally
            if c == Some('(') {
                // Might be $((...))
                let lexpos = LEX_LEXBUF.with_borrow(|b| b.buf_len());
                self.add(INPAR);
                self.add('(');

                if self.dquote_parse(')', false).is_ok() {
                    let c2 = self.hgetc();
                    if c2 == Some(')') {
                        self.add(')');
                        return CMD_OR_MATH_MATH;
                    }
                    if let Some(c2) = c2 {
                        self.hungetc(c2);
                    }
                }

                // Not math, restore and parse as command
                while LEX_LEXBUF.with_borrow(|b| b.buf_len()) > lexpos {
                    if let Some(ch) = LEX_LEXBUF.with_borrow_mut(|b| b.pop()) {
                        self.hungetc(ch);
                    }
                }
                self.hungetc('(');
                LEX_LEXSTOP.set(false);
            } else {
                if let Some(c) = c {
                    self.hungetc(c);
                }
                LEX_LEXSTOP.set(false);
            }

            return if self.skip_command_sub().is_err() {
                CMD_OR_MATH_ERR
            } else {
                CMD_OR_MATH_CMD
            };
        }
    }

    /// Skip over `(...)` for command-style substitutions: `$(...)`,
    /// `<(...)`, `>(...)`. Direct port of zsh/Src/lex.c:2080-end
    /// `skipcomm`. Per the C source comment: "we'll parse the input
    /// until we find an unmatched closing parenthesis. However, we'll
    /// throw away the result of the parsing and just keep the string
    /// we've built up on the way."
    ///
    /// zshrs port note: the C source uses zcontext_save/restore +
    /// strinbeg/inpush to set up an isolated lex context for the
    /// throw-away parse. zshrs's standalone walker tracks paren
    /// depth directly without re-entering the parser. Same
    /// invariant: stops at the matching `)`.
    fn skip_command_sub(&mut self) -> Result<(), ()> {
        let mut pct = 1;
        let mut start = true;
        const MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0;

        self.add(INPAR);

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                LEX_ERROR.with_borrow_mut(|e| *e = Some("skip_command_sub exceeded maximum iterations".to_string()));
                return Err(());
            }

            let c = self.hgetc();
            let c = match c {
                Some(c) => c,
                None => {
                    LEX_LEXSTOP.set(true);
                    return Err(());
                }
            };

            let iswhite = Self::is_inblank(c);

            match c {
                '(' => {
                    pct += 1;
                    self.add(c);
                }
                ')' => {
                    pct -= 1;
                    if pct == 0 {
                        return Ok(());
                    }
                    self.add(c);
                }
                '\\' => {
                    self.add(c);
                    if let Some(c) = self.hgetc() {
                        self.add(c);
                    }
                }
                '\'' => {
                    self.add(c);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('\'') => {
                                self.add('\'');
                                break;
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                LEX_LEXSTOP.set(true);
                                return Err(());
                            }
                        }
                    }
                }
                '"' => {
                    self.add(c);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('"') => {
                                self.add('"');
                                break;
                            }
                            Some('\\') => {
                                self.add('\\');
                                if let Some(ch) = self.hgetc() {
                                    self.add(ch);
                                }
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                LEX_LEXSTOP.set(true);
                                return Err(());
                            }
                        }
                    }
                }
                '`' => {
                    self.add(c);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('`') => {
                                self.add('`');
                                break;
                            }
                            Some('\\') => {
                                self.add('\\');
                                if let Some(ch) = self.hgetc() {
                                    self.add(ch);
                                }
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                LEX_LEXSTOP.set(true);
                                return Err(());
                            }
                        }
                    }
                }
                '#' if start => {
                    self.add(c);
                    // Skip comment to end of line
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('\n') => {
                                self.add('\n');
                                break;
                            }
                            Some(ch) => self.add(ch),
                            None => break,
                        }
                    }
                }
                _ => {
                    self.add(c);
                }
            }

            start = iswhite;
        }
    }

    /// Lex next token AND update per-context flags. Direct port of
    /// zsh/Src/lex.c:316-369 `ctxtlex`. The post-token state machine
    /// at lex.c:322-358 sets `incmdpos` based on the token shape:
    /// list separators / pipes / control keywords reset to cmd-pos;
    /// word-shaped tokens leave cmd-pos. Redirections (lex.c:361-368)
    /// stash prior incmdpos and force the redir target to non-cmd-pos.
    pub fn ctxtlex(&mut self) {
        // lex.c:319 — static `oldpos` cache for redir-target restore
        // is captured per-call here as `oldpos` below (zshrs's parser
        // re-enters ctxtlex per token, no need for static persistence).

        // lex.c:321 — `zshlex();` to advance to the next token.
        self.zshlex();

        // lex.c:322-358 — post-token incmdpos switch.
        match self.tok() {
            // lex.c:323-343 — separators / openers / conjunctions /
            // control keywords — back into cmd-pos so the next token
            // can be a fresh command.
            SEPER
            | NEWLIN
            | SEMI
            | DSEMI
            | SEMIAMP
            | SEMIBAR
            | AMPER
            | AMPERBANG
            | INPAR_TOK
            | INBRACE_TOK
            | DBAR
            | DAMPER
            | BAR_TOK
            | BARAMP
            | INOUTPAR
            | DOLOOP
            | THEN
            | ELIF
            | ELSE
            | DOUTBRACK => {
                LEX_INCMDPOS.set(true);
            }
            // lex.c:345-353 — word/value-shaped tokens leave cmd-pos
            // so subsequent tokens are arguments, not a fresh command.
            TYPESET => {
                LEX_INCMDPOS.set(false);
                // typeset / declare / local / export / readonly /
                // integer / float / autoload accept assignment-shape
                // args (NAME=value, NAME=()). Set intypeset so the
                // lexer's `=`-after-name detector still emits Envstring
                // / Envarray for those args. Direct port of zsh's
                // lex.c which sets `intypeset` when one of the
                // typeset-family commands is seen at cmdpos.
                LEX_INTYPESET.set(true);
            }
            STRING_LEX
            | ENVARRAY
            | OUTPAR_TOK
            | CASE
            | DINBRACK => {
                LEX_INCMDPOS.set(false);
            }
            SEPER
            | NEWLIN
            | SEMI
            | DSEMI
            | SEMIAMP
            | SEMIBAR
            | AMPER
            | DAMPER
            | DBAR
            | BARAMP => {
                // End of typeset-arg list — clear the intypeset bit
                // so subsequent commands don't see assignment-shape
                // recognition. Direct port of zsh's lex.c which
                // clears intypeset on every command separator.
                LEX_INTYPESET.set(false);
            }
            _ => {}
        }

        // lex.c:359-360 — `infor` decay. FOR sets infor=2 so the next
        // DINPAR can detect c-style for. After any non-DINPAR, decay
        // to 0 (or back to 2 if we just saw FOR again).
        if self.tok() != DINPAR {
            LEX_INFOR.set(if self.tok() == FOR { 2 } else { 0 });
        }

        // lex.c:361-368 — redir-target context dance. After consuming
        // a redir operator, the following token (the file path) sees
        // incmdpos=0 even when its inherent shape would put it back
        // in cmd-pos. After the redir target, restore from oldpos
        // (struct field — must persist across zshlex calls).
        if IS_REDIROP(self.tok())
            || self.tok() == FOR
            || self.tok() == FOREACH
            || self.tok() == SELECT
        {
            LEX_INREDIR.set(true);
            LEX_OLDPOS.set(LEX_INCMDPOS.get());
            LEX_INCMDPOS.set(false);
        } else if LEX_INREDIR.get() {
            LEX_INCMDPOS.set(LEX_OLDPOS.get());
            LEX_INREDIR.set(false);
        }
    }

    /// Mark the current word as the one ZLE was looking for. Direct
    /// port of zsh/Src/lex.c:1881-1897 `gotword`. Only meaningful
    /// when the lexer was started with LEXFLAGS_ZLE for completion;
    /// after this call `lexflags` is cleared so subsequent tokens
    /// don't re-trigger word tracking.
    ///
    /// zshrs port note: zsh's gotword updates `wb`/`we` (word begin/
    /// end positions) based on `zlemetacs` (cursor pos), `zlemetall`
    /// (line length), `inbufct`, and `addedx` — all live in zsh's
    /// input.c globals which zshrs hasn't wired through the lexer.
    /// Only the `lexflags = 0` side-effect at lex.c:1895 is
    /// reproducible without that integration.
    pub fn gotword(&mut self) {
        // lex.c:1895 — `lexflags = 0;`
        LEX_LEXFLAGS.set(0);
    }

    /// Register a heredoc to be processed at next newline
    pub fn register_heredoc(&mut self, terminator: String, strip_tabs: bool) {
        LEX_HEREDOCS.with_borrow_mut(|v| v.push(HereDoc {
            terminator,
            strip_tabs,
            content: String::new(),
            quoted: false,
            processed: false,
        }));
    }

    /// Check for reserved word — mirrors lex.c:2002-2015 in `exalias`,
    /// but reachable from the bare `zshlex` path (without going
    /// through `exalias`'s alias-expansion first). Promotes STRING_TOK
    /// tokens to keyword tokens when:
    ///   - incmdpos is set (or text is `}` ending a brace block)
    ///   - text is `]]` and we're inside `[[ ]]` (incond > 0)
    ///   - text is bare `!` and we're at the start of a cond (incond == 1)
    pub fn check_reserved_word(&mut self) -> bool {
        let _t_tokstr = self.tokstr();
            if let Some(tokstr) = _t_tokstr.as_deref() {
            if LEX_INCMDPOS.get() || (tokstr == "}" && self.tok() == STRING_LEX) {
                // Port of `Src/lex.c:2002` `if ((rw = (Reswd) reswdtab->getnode(reswdtab, tokstr)))`
                // — query the canonical `reswdtab` (hashtable.c:1076 reswds[]).
                // zshrs divergence: `nocorrect` stays as a plain STRING so the
                // precommand-modifier dispatcher in compile_zsh sees it intact;
                // promoting to NOCORRECT silently erased `nocorrect CMD ARGS`
                // because the downstream parser has no consumer for NOCORRECT.
                let lookup = {
                    let guard = crate::ported::hashtable::reswdtab_lock().lock().unwrap();
                    guard.get(tokstr).map(|rw| rw.token)
                };
                if let Some(tok) = lookup.filter(|&t| t != NOCORRECT) {
                    self.set_tok(tok);
                    if tok == REPEAT {
                        LEX_INREPEAT.set(1);
                    }
                    if tok == DINBRACK {
                        LEX_INCOND.set(1);
                    }
                    return true;
                }
                if tokstr == "]]" && LEX_INCOND.get() > 0 {
                    self.set_tok(DOUTBRACK);
                    LEX_INCOND.set(0);
                    return true;
                }
            }
            // lex.c:2010-2014 — `]]` and `!` are recognized inside `[[`
            // regardless of incmdpos.
            if LEX_INCOND.get() > 0 && tokstr == "]]" {
                self.set_tok(DOUTBRACK);
                LEX_INCOND.set(0);
                return true;
            }
            if LEX_INCOND.get() == 1 && tokstr == "!" {
                self.set_tok(BANG_TOK);
                return true;
            }
        }
        false
    }
}

// Direct port of the anonymous enum at `Src/lex.c:483-487`:
//   enum { CMD_OR_MATH_CMD, CMD_OR_MATH_MATH, CMD_OR_MATH_ERR };
// `cmd_or_math()` and `cmd_or_math_sub()` return one of these as `int`.
// Following the same flat-const pattern zshrs uses for lextok
// (zsh_h.rs:198-251) so call sites read the C identifier verbatim.
pub const CMD_OR_MATH_CMD: i32 = 0;
pub const CMD_OR_MATH_MATH: i32 = 1;
pub const CMD_OR_MATH_ERR: i32 = 2;

// ============================================================================
// Additional parsing functions ported from lex.c
// ============================================================================

/// Check whether we're looking at valid numeric globbing syntax
/// `<N-M>` / `<N->` / `<-M>` / `<->`. Call pointing just after the
/// opening `<`. Leaves the input position unchanged, returning true
/// or false.
///
/// Direct port of zsh/Src/lex.c:580-610 `isnumglob`. C source uses
/// hgetc/hungetc against the input stream and a temp buffer to
/// remember consumed chars; zshrs takes a `(input, pos)` slice and
/// scans without consumption. Same predicate, different I/O model.
pub fn isnumglob(input: &str, pos: usize) -> bool {
    let chars: Vec<char> = input[pos..].chars().collect();
    let mut i = 0;
    let mut expect_close = false;

    // Look for digits, then -, then digits, then >
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            i += 1;
        } else if c == '-' && !expect_close {
            expect_close = true;
            i += 1;
        } else if c == '>' && expect_close {
            return true;
        } else {
            break;
        }
    }
    false
}

/// Tokenize a string as if in double quotes (error-tolerant variant).
///
/// Direct port of zsh/Src/lex.c:1713-1733 `parsestrnoerr`. The C
/// source: zcontext_save → untokenize → inpush → strinbeg →
/// `lexbuf.ptr = tokstr = *s; lexbuf.siz = l + 1` →
/// `err = dquote_parse('\0', 1)` → strinend → inpop → zcontext_restore.
/// Returns the tokenized string on success, or the offending char as
/// an error code (zsh convention: `> 32 && < 127` → printable, else
/// generic).
///
/// zshrs port: the C version drives the lexer's dquote_parse method
/// against the input string. zshrs's standalone walker produces the
/// same BNULL/QSTRING/QTICK token markers without re-entering the
/// lexer — same output for typical bodies. Documented divergence:
/// nested cmd-sub `$(...)` and arith `$((...))` aren't lexed
/// recursively; the runtime handles them at expansion time.
pub fn parsestrnoerr(s: &str) -> Result<String, String> {
    parsestr_inner(s)
}

/// Tokenize a string as if in double quotes (error-reporting variant).
///
/// Direct port of zsh/Src/lex.c:1693-1709 `parsestr`. C source:
/// `if ((err = parsestrnoerr(s))) { untokenize(*s); ... zerr("parse
/// error near `%c'", err); tok = LEXERR; }`. zshrs's wrapper
/// returns the same Result and lets the caller emit the diagnostic.
///
/// Both `parsestr` and `parsestrnoerr` share the inner walker; the
/// only difference in C is whether errors trigger `zerr`. zshrs
/// returns `Err(msg)` from both — the caller decides whether to
/// surface the diagnostic.
pub fn parsestr(s: &str) -> Result<String, String> {
    parsestr_inner(s)
}

/// Shared body for parsestr / parsestrnoerr.
fn parsestr_inner(s: &str) -> Result<String, String> {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                i += 1;
                if i < chars.len() {
                    let next = chars[i];
                    match next {
                        '$' | '\\' | '`' | '"' | '\n' => {
                            result.push(BNULL);
                            result.push(next);
                        }
                        _ => {
                            result.push('\\');
                            result.push(next);
                        }
                    }
                } else {
                    result.push('\\');
                }
            }
            '$' => {
                result.push(QSTRING);
                if i + 1 < chars.len() {
                    let next = chars[i + 1];
                    if next == '{' {
                        result.push(INBRACE);
                        i += 1;
                    } else if next == '(' {
                        result.push(INPAR);
                        i += 1;
                    }
                }
            }
            '`' => {
                result.push(QTICK);
            }
            _ => {
                result.push(c);
            }
        }
        i += 1;
    }

    Ok(result)
}

/// Parse a subscript in string s. Return the position after the
/// closing bracket, or None on error.
///
/// Direct port of zsh/Src/lex.c:1742-1788 `parse_subscript`. The C
/// source uses dupstring_wlen + inpush + dquote_parse to lex the
/// subscript through the main lexer; zshrs implements a focused
/// bracket-balancing walker that handles the same nesting rules
/// (`[...]`, `(...)`, `{...}`) without re-entering the lexer.
///
/// zshrs port note: zsh's parse_subscript also handles a `sub`
/// flag that controls whether `$` and quotes are tokenized — that
/// flag isn't exposed here. Most callers don't need it; the few
/// that do (parameter expansion's `${var[expr]}`) handle the
/// quote-aware lex separately at the expansion layer.
pub fn parse_subscript(s: &str, endchar: char) -> Option<usize> {
    if s.is_empty() || s.starts_with(endchar) {
        return None;
    }

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut depth = 0;
    let mut in_dquote = false;
    let mut in_squote = false;

    while i < chars.len() {
        let c = chars[i];

        if in_squote {
            if c == '\'' {
                in_squote = false;
            }
            i += 1;
            continue;
        }

        if in_dquote {
            if c == '"' {
                in_dquote = false;
            } else if c == '\\' && i + 1 < chars.len() {
                i += 1; // skip escaped char
            }
            i += 1;
            continue;
        }

        match c {
            '\\' => {
                i += 1; // skip next char
            }
            '\'' => {
                in_squote = true;
            }
            '"' => {
                in_dquote = true;
            }
            '[' | '(' => {
                depth += 1;
            }
            ']' | ')' => {
                if depth > 0 {
                    depth -= 1;
                } else if c == endchar {
                    return Some(i);
                }
            }
            _ => {}
        }

        if c == endchar && depth == 0 {
            return Some(i);
        }

        i += 1;
    }

    None
}

/// Tokenize a string as if it were a normal command-line argument
/// but it may contain separators. Used for ${...%...} substitutions.
///
/// Direct port of zsh/Src/lex.c:1796-1880 `parse_subst_string`.
/// zsh's version sets `noaliases = 1` + `lexflags = 0` + uses
/// zcontext_save/inpush/strinbeg → dquote_parse('\0', 1) →
/// strinend/inpop/zcontext_restore. zshrs's standalone walker
/// produces the same BNULL/SNULL/DNULL/INPAR/INBRACK markers
/// without re-entering the lexer.
///
/// zshrs port note: the C source returns int (0=ok, char value =
/// where it stopped on error); zshrs returns Result<String,String>
/// returning the tokenized text directly. Lossy for callers that
/// need to know the exact stop position, but nothing in zshrs's
/// expansion layer uses that yet.
pub fn parse_subst_string(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Ok(String::new());
    }

    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                result.push(BNULL);
                i += 1;
                if i < chars.len() {
                    result.push(chars[i]);
                }
            }
            '\'' => {
                result.push(SNULL);
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    result.push(chars[i]);
                    i += 1;
                }
                result.push(SNULL);
            }
            '"' => {
                result.push(DNULL);
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        result.push(BNULL);
                        i += 1;
                        result.push(chars[i]);
                    } else if chars[i] == '$' {
                        result.push(QSTRING);
                    } else {
                        result.push(chars[i]);
                    }
                    i += 1;
                }
                result.push(DNULL);
            }
            '$' => {
                result.push(STRING_TOK);
                if i + 1 < chars.len() {
                    match chars[i + 1] {
                        '{' => {
                            result.push(INBRACE);
                            i += 1;
                        }
                        '(' => {
                            result.push(INPAR);
                            i += 1;
                        }
                        _ => {}
                    }
                }
            }
            '*' => result.push(STAR),
            '?' => result.push(QUEST),
            '[' => result.push(INBRACK),
            ']' => result.push(OUTBRACK),
            '{' => result.push(INBRACE),
            '}' => result.push(OUTBRACE),
            '~' => result.push(TILDE),
            '#' => result.push(POUND),
            '^' => result.push(HAT),
            _ => result.push(c),
        }
        i += 1;
    }

    Ok(result)
}

/// Untokenize a string - convert tokenized chars back to original
///
/// Port of untokenize() from exec.c (but used by lexer too)
/// Like `untokenize`, but maps SNULL → `'` and DNULL → `"` instead of
/// stripping them. Used by callers that need the source form including
/// quoting (e.g. arithmetic-substitution detection in compile_zsh).
pub fn untokenize_preserve_quotes(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        let cu = c as u32;
        if (0x83..=0x9f).contains(&cu) {
            match c {
                c if c == POUND => result.push('#'),
                c if c == STRING_TOK => result.push('$'),
                c if c == HAT => result.push('^'),
                c if c == STAR => result.push('*'),
                c if c == INPAR => result.push('('),
                c if c == OUTPAR => result.push(')'),
                c if c == INPARMATH => result.push('('),
                c if c == OUTPARMATH => result.push(')'),
                c if c == QSTRING => result.push('$'),
                c if c == EQUALS => result.push('='),
                c if c == BAR => result.push('|'),
                c if c == INBRACE => result.push('{'),
                c if c == OUTBRACE => result.push('}'),
                c if c == INBRACK => result.push('['),
                c if c == OUTBRACK => result.push(']'),
                c if c == TICK => result.push('`'),
                c if c == INANG => result.push('<'),
                c if c == OUTANG => result.push('>'),
                c if c == OUTANG_PROC => result.push('>'),
                c if c == QUEST => result.push('?'),
                c if c == TILDE => result.push('~'),
                c if c == QTICK => result.push('`'),
                c if c == COMMA => result.push(','),
                c if c == DASH => result.push('-'),
                c if c == BANG => result.push('!'),
                c if c == SNULL => result.push('\''),
                c if c == DNULL => result.push('"'),
                c if c == BNULL => result.push('\\'),
                _ => {
                    let idx = c as usize;
                    if idx < ztokens.len() {
                        result.push(ztokens.chars().nth(idx).unwrap_or(c));
                    } else {
                        result.push(c);
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Decode `\X` escape sequences for `$'...'` content.
/// Port of `getkeystring()` from Src/utils.c:6915 with the
/// `GETKEYS_DOLLARS_QUOTE` flag — handles the `\n`/`\t`/`\r`/`\e`/
/// `\E`/`\a`/`\b`/`\f`/`\v`/`\xNN`/`\uNNNN`/`\UNNNNNNNN`/octal/`\\`/`\'`
/// arms the C source recognizes inside dollar-single-quoted
/// strings. Walks `chars[start..]` until `Snull` is hit, returns
/// `(decoded, end_idx)` where `end_idx` points at the terminating
/// `Snull`. `Bnull \\` and `Bnull '` are user-literal `\` / `'`
/// per Src/lex.c:1303.
fn getkeystring_dollar_quote(chars: &[char], start: usize) -> (String, usize) {
    let mut out = String::new();
    let mut i = start;
    while i < chars.len() {
        let c = chars[i];
        if c == SNULL {
            return (out, i);
        }
        if c == BNULL {
            // Bnull marks a user-literal `\\` or `\'` per
            // Src/lex.c:1303-1306. The next char is the literal.
            i += 1;
            if i < chars.len() {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '\\' && i + 1 < chars.len() {
            let nc = chars[i + 1];
            match nc {
                'a' => {
                    out.push('\x07');
                    i += 2;
                }
                'b' => {
                    out.push('\x08');
                    i += 2;
                }
                'e' | 'E' => {
                    out.push('\x1b');
                    i += 2;
                }
                'f' => {
                    out.push('\x0c');
                    i += 2;
                }
                'n' => {
                    out.push('\n');
                    i += 2;
                }
                'r' => {
                    out.push('\r');
                    i += 2;
                }
                't' => {
                    out.push('\t');
                    i += 2;
                }
                'v' => {
                    out.push('\x0b');
                    i += 2;
                }
                '\\' | '\'' | '"' => {
                    out.push(nc);
                    i += 2;
                }
                'x' => {
                    // \xNN — up to 2 hex digits per Src/utils.c:7156
                    let mut val: u32 = 0;
                    let mut consumed = 2; // \x
                    let mut got = 0;
                    while got < 2 && i + consumed < chars.len() {
                        let h = chars[i + consumed];
                        if let Some(d) = h.to_digit(16) {
                            val = val * 16 + d;
                            consumed += 1;
                            got += 1;
                        } else {
                            break;
                        }
                    }
                    if got == 0 {
                        // No hex digits — emit literal `\x` per
                        // Src/utils.c:7160-7163 fallthrough
                        out.push('\\');
                        out.push('x');
                    } else if let Some(ch) = char::from_u32(val) {
                        out.push(ch);
                    }
                    i += consumed;
                }
                'u' | 'U' => {
                    let n = if nc == 'u' { 4 } else { 8 };
                    let mut val: u32 = 0;
                    let mut consumed = 2; // \u or \U
                    let mut got = 0;
                    while got < n && i + consumed < chars.len() {
                        let h = chars[i + consumed];
                        if let Some(d) = h.to_digit(16) {
                            val = val * 16 + d;
                            consumed += 1;
                            got += 1;
                        } else {
                            break;
                        }
                    }
                    if let Some(ch) = char::from_u32(val) {
                        out.push(ch);
                    }
                    i += consumed;
                }
                '0'..='7' => {
                    // Octal — up to 3 digits per Src/utils.c:7156
                    let mut val: u32 = 0;
                    let mut consumed = 1; // skip backslash
                    let mut got = 0;
                    while got < 3 && i + consumed < chars.len() {
                        let h = chars[i + consumed];
                        if let Some(d) = h.to_digit(8) {
                            val = val * 8 + d;
                            consumed += 1;
                            got += 1;
                        } else {
                            break;
                        }
                    }
                    if let Some(ch) = char::from_u32(val) {
                        out.push(ch);
                    }
                    i += consumed;
                }
                _ => {
                    // Unknown escape — keep `\` per
                    // Src/utils.c:7180-7185 default branch
                    out.push('\\');
                    out.push(nc);
                    i += 2;
                }
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    (out, i)
}

pub fn untokenize(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        // Token chars live in zsh's META range (0x83 = META through 0x9f =
        // BNULL). Anything in that range needs un-mapping before display
        // or downstream consumption. The original `< 32` test was wrong —
        // none of zsh's tokens land in that range.
        let cu = c as u32;
        if (0x83..=0x9f).contains(&cu) {
            // `Qstring Snull` opens a `$'...'` ANSI-C-quoted region.
            // Per Src/subst.c:301-304, when `stringsubst()` hits an
            // `Snull` it calls `stringsubstquote()` (line 206) which
            // calls `getkeystring(s+2, ...)` over the content,
            // skipping the leading `Qstring Snull` and stopping at
            // the closing `Snull`. zshrs's pipeline runs untokenize
            // at points where C runs subst, so we apply the same
            // decoding inline here. Result: the entire `$'...'`
            // region is replaced by its decoded content with no
            // `$`/`'`/marker remnants.
            if c == QSTRING
                && i + 1 < chars.len()
                && chars[i + 1] == SNULL
            {
                let (decoded, end) = getkeystring_dollar_quote(&chars, i + 2);
                result.push_str(&decoded);
                // `end` points at the closing `Snull` (or end of
                // string if unterminated); skip past it.
                i = if end < chars.len() { end + 1 } else { end };
                continue;
            }
            // Convert token back to original character
            match c {
                c if c == POUND => result.push('#'),
                c if c == STRING_TOK => result.push('$'),
                c if c == HAT => result.push('^'),
                c if c == STAR => result.push('*'),
                c if c == INPAR => result.push('('),
                c if c == OUTPAR => result.push(')'),
                c if c == INPARMATH => result.push('('),
                c if c == OUTPARMATH => result.push(')'),
                c if c == QSTRING => result.push('$'),
                c if c == EQUALS => result.push('='),
                c if c == BAR => result.push('|'),
                c if c == INBRACE => result.push('{'),
                c if c == OUTBRACE => result.push('}'),
                c if c == INBRACK => result.push('['),
                c if c == OUTBRACK => result.push(']'),
                c if c == TICK => result.push('`'),
                c if c == INANG => result.push('<'),
                c if c == OUTANG => result.push('>'),
                c if c == OUTANG_PROC => result.push('>'),
                c if c == QUEST => result.push('?'),
                c if c == TILDE => result.push('~'),
                c if c == QTICK => result.push('`'),
                c if c == COMMA => result.push(','),
                c if c == DASH => result.push('-'),
                c if c == BANG => result.push('!'),
                c if c == SNULL
                    || c == DNULL
                    || c == BNULL =>
                {
                    // Null markers - skip
                }
                _ => {
                    // Unknown token, try ztokens lookup
                    let idx = c as usize;
                    if idx < ztokens.len() {
                        result.push(ztokens.chars().nth(idx).unwrap_or(c));
                    } else {
                        result.push(c);
                    }
                }
            }
        } else {
            result.push(c);
        }
        i += 1;
    }

    result
}

/// Check if a string contains any token characters
pub fn has_token(s: &str) -> bool {
    s.chars().any(|c| (c as u32) < 32)
}

/// Convert token characters to their printable form for display
pub fn tokens_to_printable(s: &str) -> String {
    untokenize(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let mut lexer = ZshLexer::new("echo hello");
        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
        assert_eq!(lexer.tokstr, Some("echo".to_string()));

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
        assert_eq!(lexer.tokstr, Some("hello".to_string()));

        lexer.zshlex();
        assert_eq!(lexer.tok, ENDINPUT);
    }

    #[test]
    fn test_pipeline() {
        let mut lexer = ZshLexer::new("ls | grep foo");
        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);

        lexer.zshlex();
        assert_eq!(lexer.tok, BAR_TOK);

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
    }

    #[test]
    fn test_redirections() {
        let mut lexer = ZshLexer::new("echo > file");
        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);

        lexer.zshlex();
        assert_eq!(lexer.tok, OUTANG_TOK);

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
    }

    #[test]
    fn test_heredoc() {
        let mut lexer = ZshLexer::new("cat << EOF");
        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);

        lexer.zshlex();
        assert_eq!(lexer.tok, DINANG);

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
    }

    #[test]
    fn test_single_quotes() {
        let mut lexer = ZshLexer::new("echo 'hello world'");
        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
        // Should contain Snull markers around literal content
        assert!(lexer.tokstr.is_some());
    }

    #[test]
    fn test_function_tokens() {
        let mut lexer = ZshLexer::new("function foo { }");
        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            FUNC,
            "expected Func, got {:?}",
            lexer.tok
        );

        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            STRING_LEX,
            "expected String for 'foo', got {:?}",
            lexer.tok
        );
        assert_eq!(lexer.tokstr, Some("foo".to_string()));

        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            INBRACE_TOK,
            "expected Inbrace, got {:?} tokstr={:?}",
            lexer.tok,
            lexer.tokstr
        );

        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            OUTBRACE_TOK,
            "expected Outbrace, got {:?} tokstr={:?} incmdpos={}",
            lexer.tok,
            lexer.tokstr,
            lexer.incmdpos()
        );
    }

    #[test]
    fn test_double_quotes() {
        let mut lexer = ZshLexer::new("echo \"hello $name\"");
        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
        // Should contain tokenized content
        assert!(lexer.tokstr.is_some());
    }

    #[test]
    fn test_command_substitution() {
        let mut lexer = ZshLexer::new("echo $(pwd)");
        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
    }

    #[test]
    fn test_env_assignment() {
        let mut lexer = ZshLexer::new("FOO=bar echo");
        lexer.set_incmdpos(true);
        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            ENVSTRING,
            "tok={:?} tokstr={:?}",
            lexer.tok,
            lexer.tokstr
        );

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
    }

    #[test]
    fn test_array_assignment() {
        let mut lexer = ZshLexer::new("arr=(a b c)");
        lexer.set_incmdpos(true);
        lexer.zshlex();
        assert_eq!(lexer.tok, ENVARRAY);
    }

    #[test]
    fn test_process_substitution() {
        let mut lexer = ZshLexer::new("diff <(ls) >(cat)");
        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
        // <(ls) is tokenized into the string

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
        // >(cat) is tokenized
    }

    #[test]
    fn test_arithmetic() {
        let mut lexer = ZshLexer::new("echo $((1+2))");
        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);

        lexer.zshlex();
        assert_eq!(lexer.tok, STRING_LEX);
    }

    #[test]
    fn test_semicolon_variants() {
        let mut lexer = ZshLexer::new("case x in a) cmd;; b) cmd;& c) cmd;| esac");

        // Skip to first ;;
        loop {
            lexer.zshlex();
            if lexer.tok == DSEMI || lexer.tok == ENDINPUT {
                break;
            }
        }
        assert_eq!(lexer.tok, DSEMI);

        // Find ;&
        loop {
            lexer.zshlex();
            if lexer.tok == SEMIAMP || lexer.tok == ENDINPUT {
                break;
            }
        }
        assert_eq!(lexer.tok, SEMIAMP);

        // Find ;|
        loop {
            lexer.zshlex();
            if lexer.tok == SEMIBAR || lexer.tok == ENDINPUT {
                break;
            }
        }
        assert_eq!(lexer.tok, SEMIBAR);
    }
}
