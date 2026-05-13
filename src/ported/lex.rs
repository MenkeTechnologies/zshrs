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
// (Pound/Stringg/Inpar/…), REDIR_* and COND_* constants live as flat
// `pub const`s in `super::zsh_h` per `Src/zsh.h:144-679` and are used from
// here directly — do NOT wrap them in Rust enums or sub-modules.
// =============================================================================

// Character tokens — port of `Src/zsh.h:144-224` `#define Pound … #define
// Marker`. Imported with the zsh_h.rs disambiguation (Stringg → Stringg,
// OutangProc → OutangProc) so the lex.rs body keeps the original C-style
// short names without colliding with `STRING_LEX` (the lextok=34 constant).
use crate::ported::prompt::{cmdpop, cmdpush};
use crate::ported::zsh_h::{
    isset, unset, Bang, Bar, Bnull, Bnullkeep, Comma, Dash, Dnull, Equals, Hat, Inang, Inbrace,
    Inbrack, Inpar, Inparmath, Marker, Nularg, Outang, OutangProc, Outbrace, Outbrack, Outpar,
    Outparmath, Pound, Qstring, Qtick, Quest, Snull, Star, Stringg, Tick, Tilde, ALIASESOPT,
    CORRECT, CORRECTALL, CSHJUNKIEQUOTES, CS_BQUOTE, CS_BRACE, CS_BRACEPAR, CS_CMDSUBST, CS_CURSH,
    CS_DQUOTE, CS_HEREDOC, CS_HEREDOCD, CS_MATH, CS_MATHSUBST, CS_QUOTE, HISTALLOWCLOBBER,
    IGNOREBRACES, IGNORECLOSEBRACES, INTERACTIVECOMMENTS, KSHGLOB, META, POSIXALIASES, RCQUOTES,
    SHGLOB, SHINSTDIN, SHORTLOOPS, SHORTREPEAT,
};

use crate::zsh_h::lex_stack;
use crate::ztype_h::itok;

/// Port of `static const char ztokens[]` from `Src/lex.c:80`.
pub const ztokens: &str = "#$^*(())$=|{}[]`<>>?~`,-!'\"\\\\";

// `enum lextok` — port of `Src/zsh.h:304-371`. The full constant set
// (`NULLTOK`, `SEPER`, …, `TYPESET`) and the `lextok` type alias live
// in `super::zsh_h:198-262`. Re-export here so external callers can
// keep saying `lex::lextok` / `tokens::lextok` without reaching into
// `zsh_h::` directly. `IS_REDIROP()` (port of `Src/zsh.h:408`
// `#define IS_REDIROP`) lives in `zsh_h:318`.
pub use super::zsh_h::{
    lextok, AMPER, AMPERBANG, AMPOUTANG, BANG_TOK, BARAMP, BAR_TOK, CASE, COPROC, DAMPER, DBAR,
    DINANG, DINANGDASH, DINBRACK, DINPAR, DOLOOP, DONE, DOUTANG, DOUTANGAMP, DOUTANGAMPBANG,
    DOUTANGBANG, DOUTBRACK, DOUTPAR, DSEMI, ELIF, ELSE, ENDINPUT, ENVARRAY, ENVSTRING, ESAC, FI,
    FOR, FOREACH, FUNC, IF, INANGAMP, INANG_TOK, INBRACE_TOK, INOUTANG, INOUTPAR, INPAR_TOK,
    IS_REDIROP, LEXERR, NEWLIN, NOCORRECT, NULLTOK, OUTANGAMP, OUTANGAMPBANG, OUTANGBANG,
    OUTANG_TOK, OUTBRACE_TOK, OUTPAR_TOK, REPEAT, SELECT, SEMI, SEMIAMP, SEMIBAR, SEPER,
    STRING_LEX, THEN, TIME, TRINANG, TYPESET, UNTIL, WHILE, ZEND,
};

// RedirType / CondType — flat `REDIR_*` (`Src/zsh.h:377-408`) and
// `COND_*` (`Src/zsh.h:660-679`) constants already live in
// `super::zsh_h`. Do NOT wrap them in Rust enums here — the wrapper
// is a fake abstraction (no C counterpart).
//
// LX1_* / LX2_* — flat `#define`s in `Src/lex.c:371-405`. The
// lexer's `gettok` body uses these as the action table for the
// first / second-tier character dispatch.

/// `#define LX1_BKSLASH 0` (Src/lex.c:371).
pub const LX1_BKSLASH: u8 = 0;
/// `#define LX1_COMMENT 1` (Src/lex.c:372).
pub const LX1_COMMENT: u8 = 1;
/// `#define LX1_NEWLIN 2` (Src/lex.c:373).
pub const LX1_NEWLIN: u8 = 2;
/// `#define LX1_SEMI 3` (Src/lex.c:374).
pub const LX1_SEMI: u8 = 3;
/// `#define LX1_AMPER 5` (Src/lex.c:375).
pub const LX1_AMPER: u8 = 5;
/// `#define LX1_BAR 6` (Src/lex.c:376).
pub const LX1_BAR: u8 = 6;
/// `#define LX1_INPAR 7` (Src/lex.c:377).
pub const LX1_INPAR: u8 = 7;
/// `#define LX1_OUTPAR 8` (Src/lex.c:378).
pub const LX1_OUTPAR: u8 = 8;
/// `#define LX1_INANG 13` (Src/lex.c:379).
pub const LX1_INANG: u8 = 13;
/// `#define LX1_OUTANG 14` (Src/lex.c:380).
pub const LX1_OUTANG: u8 = 14;
/// `#define LX1_OTHER 15` (Src/lex.c:381).
pub const LX1_OTHER: u8 = 15;

/// `#define LX2_BREAK 0` (Src/lex.c:383).
pub const LX2_BREAK: u8 = 0;
/// `#define LX2_OUTPAR 1` (Src/lex.c:384).
pub const LX2_OUTPAR: u8 = 1;
/// `#define LX2_BAR 2` (Src/lex.c:385).
pub const LX2_BAR: u8 = 2;
/// `#define LX2_STRING 3` (Src/lex.c:386).
pub const LX2_STRING: u8 = 3;
/// `#define LX2_INBRACK 4` (Src/lex.c:387).
pub const LX2_INBRACK: u8 = 4;
/// `#define LX2_OUTBRACK 5` (Src/lex.c:388).
pub const LX2_OUTBRACK: u8 = 5;
/// `#define LX2_TILDE 6` (Src/lex.c:389).
pub const LX2_TILDE: u8 = 6;
/// `#define LX2_INPAR 7` (Src/lex.c:390).
pub const LX2_INPAR: u8 = 7;
/// `#define LX2_INBRACE 8` (Src/lex.c:391).
pub const LX2_INBRACE: u8 = 8;
/// `#define LX2_OUTBRACE 9` (Src/lex.c:392).
pub const LX2_OUTBRACE: u8 = 9;
/// `#define LX2_OUTANG 10` (Src/lex.c:393).
pub const LX2_OUTANG: u8 = 10;
/// `#define LX2_INANG 11` (Src/lex.c:394).
pub const LX2_INANG: u8 = 11;
/// `#define LX2_EQUALS 12` (Src/lex.c:395).
pub const LX2_EQUALS: u8 = 12;
/// `#define LX2_BKSLASH 13` (Src/lex.c:396).
pub const LX2_BKSLASH: u8 = 13;
/// `#define LX2_QUOTE 14` (Src/lex.c:397).
pub const LX2_QUOTE: u8 = 14;
/// `#define LX2_DQUOTE 15` (Src/lex.c:398).
pub const LX2_DQUOTE: u8 = 15;
/// `#define LX2_BQUOTE 16` (Src/lex.c:399).
pub const LX2_BQUOTE: u8 = 16;
/// `#define LX2_COMMA 17` (Src/lex.c:400).
pub const LX2_COMMA: u8 = 17;
/// `#define LX2_DASH 18` (Src/lex.c:401).
pub const LX2_DASH: u8 = 18;
/// `#define LX2_BANG 19` (Src/lex.c:402).
pub const LX2_BANG: u8 = 19;
/// `#define LX2_OTHER 20` (Src/lex.c:403).
pub const LX2_OTHER: u8 = 20;
/// `#define LX2_META 21` (Src/lex.c:404).
pub const LX2_META: u8 = 21;

/// `static unsigned char lexact1[256]` from `Src/lex.c:406`. Per-byte
/// action table for the first-tier dispatch in `gettok`. Init'd by
/// `initlextabs()`.
pub static LEXACT1: std::sync::OnceLock<std::sync::Mutex<[u8; 256]>> = std::sync::OnceLock::new();
/// `static unsigned char lexact2[256]` from `Src/lex.c:406`. Per-byte
/// action table for the second-tier dispatch in `gettokstr`.
pub static LEXACT2: std::sync::OnceLock<std::sync::Mutex<[u8; 256]>> = std::sync::OnceLock::new();
/// `static unsigned char lextok2[256]` from `Src/lex.c:406`. Per-byte
/// token-character map: maps `*` → `Star`, `?` → `Quest`, etc.
pub static LEXTOK2: std::sync::OnceLock<std::sync::Mutex<[u8; 256]>> = std::sync::OnceLock::new();

/// Sentinel: true once `initlextabs()` has populated the tables.
static LEX_TABS_INITED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[inline]
fn ensure_tabs_inited() {
    if !LEX_TABS_INITED.load(std::sync::atomic::Ordering::Acquire) {
        initlextabs();
        LEX_TABS_INITED.store(true, std::sync::atomic::Ordering::Release);
    }
}

/// Table accessor for `lexact1[c]`. Mirrors C's `lexact1[STOUC(c)]`
/// macro at `Src/lex.c:725`.
#[inline]
pub fn lexact1_get(c: char) -> u8 {
    let idx = c as u32;
    if idx >= 256 {
        return LX1_OTHER;
    }
    ensure_tabs_inited();
    let table = LEXACT1.get().unwrap();
    table.lock().unwrap()[idx as usize]
}

/// Table accessor for `lexact2[c]`. Mirrors C's `lexact2[STOUC(c)]`
/// macro at `Src/lex.c:919` (the dispatch inside `gettokstr`).
#[inline]
pub fn lexact2_get(c: char) -> u8 {
    let idx = c as u32;
    if idx >= 256 {
        return LX2_OTHER;
    }
    ensure_tabs_inited();
    let table = LEXACT2.get().unwrap();
    table.lock().unwrap()[idx as usize]
}

/// Table accessor for `lextok2[c]`. Mirrors C's `lextok2[STOUC(c)]`
/// at `Src/lex.c:919+` — used to translate a glob metacharacter to
/// its byte-token form (`*` → `Star`, etc.).
#[inline]
pub fn lextok2_get(c: char) -> u8 {
    let idx = c as u32;
    if idx >= 256 {
        return c as u8;
    }
    ensure_tabs_inited();
    let table = LEXTOK2.get().unwrap();
    table.lock().unwrap()[idx as usize]
}

/// Port of `void initlextabs(void)` from `Src/lex.c:410`. Builds the
/// three byte-keyed action tables the lexer dispatches on.
///
/// `lexact1` — char → `LX1_*` action for the top-level `gettok`
/// dispatch. Default `LX1_OTHER`; the 14 chars in `"\\q\n;!&|(){}[]<>"`
/// get table-index actions in order.
///
/// `lexact2` — char → `LX2_*` action for `gettokstr`'s in-word
/// dispatch. Default `LX2_OTHER`; the 21 chars in
/// `";)|$[]~({}><=\\\\\'\"\`,-!"` map by index, plus `&` → `LX2_BREAK`
/// and the Meta byte → `LX2_META`.
///
/// `lextok2` — char → byte-token. Identity except for the 8 magic
/// chars that need byte-token replacement (`*`/`?`/`{`/`[`/`$`/`~`/
/// `#`/`^` → Star/Quest/Inbrace/Inbrack/Stringg/Tilde/Pound/Hat).
pub fn initlextabs() {
    // c:410
    use crate::ported::zsh_h::{Hat, Inbrace, Inbrack, Pound, Quest, Star, Stringg, Tilde, META};
    let a1 = LEXACT1.get_or_init(|| std::sync::Mutex::new([0u8; 256]));
    let a2 = LEXACT2.get_or_init(|| std::sync::Mutex::new([0u8; 256]));
    let t2 = LEXTOK2.get_or_init(|| std::sync::Mutex::new([0u8; 256]));
    let mut a1 = a1.lock().unwrap();
    let mut a2 = a2.lock().unwrap();
    let mut t2 = t2.lock().unwrap();
    // c:413-417 — seed defaults.
    for i in 0..256 {
        a1[i] = LX1_OTHER;
        a2[i] = LX2_OTHER;
        t2[i] = i as u8;
    }
    // c:418-419 — overwrite indexed punctuation.
    let lx1 = b"\\q\n;!&|(){}[]<>";
    for (i, &c) in lx1.iter().enumerate() {
        a1[c as usize] = i as u8;
    }
    let lx2 = b";)|$[]~({}><=\\'\"`,-!";
    for (i, &c) in lx2.iter().enumerate() {
        a2[c as usize] = i as u8;
    }
    // c:422-423 — special overrides.
    a2[b'&' as usize] = LX2_BREAK;
    a2[META as usize] = LX2_META;
    // c:424-431 — byte-token map for the 8 magic chars.
    t2[b'*' as usize] = Star as u8;
    t2[b'?' as usize] = Quest as u8;
    t2[b'{' as usize] = Inbrace as u8;
    t2[b'[' as usize] = Inbrack as u8;
    t2[b'$' as usize] = Stringg as u8;
    t2[b'~' as usize] = Tilde as u8;
    t2[b'#' as usize] = Pound as u8;
    t2[b'^' as usize] = Hat as u8;
}

// SPECCHARS / PATCHARS — port of `Src/zsh.h:228, 232`. Use
// `super::zsh_h::{SPECCHARS, PATCHARS}` directly; no duplicate here.
// IS_DASH() — port of `Src/zsh.h:242` `#define IS_DASH(x)`. Use
// `super::zsh_h::IS_DASH(c)` at call sites.

// Reserved-word table — the canonical port of `Src/hashtable.c:1076`
// `static struct reswd reswds[]` lives in `ported::hashtable::reswd_table`
// (built in `reswd_table::new()` at hashtable.rs:561 with the 31 entries
// from `reswds[]`). The lexer queries it via `reswdtab_lock()` from
// `check_reserved_word()` below; no duplicate table here.

#[cfg(test)]
mod tokens_tests {
    use crate::ported::hashtable::reswdtab_lock;
    use crate::ported::zsh_h::{
        Bnull, Dnull, Snull, DINANG, IF, IS_REDIROP, OUTANG_TOK, STRING_LEX, THEN,
    };

    #[test]
    fn test_token_values() {
        assert_eq!(Snull as u32, 0x9d);
        assert_eq!(Dnull as u32, 0x9e);
        assert_eq!(Bnull as u32, 0x9f);
    }

    #[test]
    fn test_reserved_words() {
        // Reserved-word lookup goes through the canonical `reswdtab`
        // (port of `Src/hashtable.c:1076 reswds[]`).
        let tab = reswdtab_lock().read().unwrap();
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
// siz: i32, len: i32}` — same shape as C. The LEX_LEXBUF /
// LEX_LEXBUF_RAW thread_locals use this canonical type directly.
// The convenience methods below are Rust-only
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
pub use crate::heredoc_ast::HereDoc;

// =============================================================================
// Lexer state — thread-local file-statics matching zsh's lex.c file-statics.
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
    /// `mod_export int wordbeg` (lex.c:123). The cursor-relative
    /// start index of the in-flight word, set in gettok when
    /// LEXFLAGS_ZLE is active. Consumed by `gotword`.
    pub static LEX_WORDBEG: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `mod_export int parbegin` (lex.c:126). Marks start of a
    /// `(...)` substitution under ZLE so completion can recurse in.
    /// `-1` when no substitution is open.
    pub static LEX_PARBEGIN: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
    /// `mod_export int parend` (lex.c:129). Marks end of a `(...)`
    /// substitution under ZLE. `-1` when no substitution closed.
    pub static LEX_PAREND: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
    /// `int isfirstln` (lex.c:114).
    pub static LEX_ISFIRSTLN: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// `int isfirstch` (lex.c:116).
    pub static LEX_ISFIRSTCH: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Pending heredocs — Rust-only working set until P9c reinstates
    /// the C `struct heredocs` linked-list shape (zsh.h:1152).
    pub static LEX_HEREDOCS: std::cell::RefCell<Vec<HereDoc>> = const { std::cell::RefCell::new(Vec::new()) };
    /// `struct lexbufstate lexbuf` (lex.c:210).
    pub static LEX_LEXBUF: std::cell::RefCell<lexbufstate> = const { std::cell::RefCell::new(
        lexbufstate { ptr: None, siz: 0, len: 0 }
    )};
    /// `int isnewlin` (lex.c:119).
    pub static LEX_ISNEWLIN: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `int lex_add_raw` (lex.c:161).
    pub static LEX_LEX_ADD_RAW: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `static char *tokstr_raw` (lex.c:165).
    pub static LEX_TOKSTR_RAW: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
    /// `struct lexbufstate lexbuf_raw` (lex.c:166).
    pub static LEX_LEXBUF_RAW: std::cell::RefCell<lexbufstate> = const { std::cell::RefCell::new(
        lexbufstate { ptr: None, siz: 0, len: 0 }
    )};
}

/// The Zsh Lexer.
///
// All lexer state lives in the file-scope `LEX_*` thread_local statics
// above, each one matching a `static` in `Src/lex.c`. There's no
// holder struct — callers use the free fns directly:
//
//    lex_init(input);
//    zshlex();
//   let tok =  tok();
//
// Accessor fns named after the former field identifiers (`tok()`,
// `tokstr()`, `set_tok(v)`, etc.) provide read/write into LEX_*.

// ─── Accessor fns for the LEX_* thread_locals (Src/lex.c file-statics) ───

/// C lex.c does not retain lexer error messages — `zerr(...)` prints
/// to stderr immediately and sets `errflag |= ERRFLAG_ERROR`. Callers
/// should check `crate::ported::utils::errflag` directly. These shims
/// remain for backward-compatibility callers (parse.rs:1469) and
/// always return None / no-op; remove once the parser stops calling.
pub fn error() -> Option<String> {
    None
}
pub fn set_error(_v: Option<String>) {}

pub fn toklineno() -> u64 {
    LEX_TOKLINENO.get()
}
pub fn set_toklineno(v: u64) {
    LEX_TOKLINENO.set(v);
}
pub fn tokfd() -> i32 {
    LEX_TOKFD.get()
}
pub fn set_tokfd(v: i32) {
    LEX_TOKFD.set(v);
}
pub fn isnewlin() -> i32 {
    LEX_ISNEWLIN.get()
}
pub fn set_isnewlin(v: i32) {
    LEX_ISNEWLIN.set(v);
}
pub fn inrepeat() -> i32 {
    LEX_INREPEAT.get()
}
pub fn set_inrepeat(v: i32) {
    LEX_INREPEAT.set(v);
}
pub fn infor() -> i32 {
    LEX_INFOR.get()
}
pub fn set_infor(v: i32) {
    LEX_INFOR.set(v);
}
pub fn inredir() -> bool {
    LEX_INREDIR.get()
}
pub fn set_inredir(v: bool) {
    LEX_INREDIR.set(v);
}
pub fn intypeset() -> bool {
    LEX_INTYPESET.get()
}
pub fn set_intypeset(v: bool) {
    LEX_INTYPESET.set(v);
}
pub fn lineno() -> u64 {
    LEX_LINENO.get()
}
pub fn set_lineno(v: u64) {
    LEX_LINENO.set(v);
}
pub fn incmdpos() -> bool {
    LEX_INCMDPOS.get()
}
pub fn set_incmdpos(v: bool) {
    LEX_INCMDPOS.set(v);
}
pub fn incond() -> i32 {
    LEX_INCOND.get()
}
pub fn set_incond(v: i32) {
    LEX_INCOND.set(v);
}
pub fn incasepat() -> i32 {
    LEX_INCASEPAT.get()
}
pub fn set_incasepat(v: i32) {
    LEX_INCASEPAT.set(v);
}
/// Pending-heredocs accessors. The Vec lives in LEX_HEREDOCS;
/// these helpers package the common operations so callers don't
/// touch the thread_local directly.
pub fn heredocs_take() -> Vec<HereDoc> {
    LEX_HEREDOCS.with_borrow_mut(|v| std::mem::take(v))
}
pub fn heredocs_set(v: Vec<HereDoc>) {
    LEX_HEREDOCS.with_borrow_mut(|c| *c = v);
}
pub fn heredocs_clear() {
    LEX_HEREDOCS.with_borrow_mut(|v| v.clear());
}
pub fn heredocs_is_empty() -> bool {
    LEX_HEREDOCS.with_borrow(|v| v.is_empty())
}
pub fn heredocs_len() -> usize {
    LEX_HEREDOCS.with_borrow(|v| v.len())
}
pub fn heredocs_clone() -> Vec<HereDoc> {
    LEX_HEREDOCS.with_borrow(|v| v.clone())
}
pub fn heredocs_push(h: HereDoc) {
    LEX_HEREDOCS.with_borrow_mut(|v| v.push(h));
}
/// `char *tokstr` accessors — direct port of lex.c:170 file-static.
pub fn tokstr() -> Option<String> {
    LEX_TOKSTR.with_borrow(|t| t.clone())
}
pub fn set_tokstr(v: Option<String>) {
    LEX_TOKSTR.with_borrow_mut(|t| *t = v);
}
pub fn tokstr_take() -> Option<String> {
    LEX_TOKSTR.with_borrow_mut(|t| t.take())
}
pub fn tokstr_is_some() -> bool {
    LEX_TOKSTR.with_borrow(|t| t.is_some())
}
pub fn tokstr_is_none() -> bool {
    LEX_TOKSTR.with_borrow(|t| t.is_none())
}
pub fn tokstr_eq(s: &str) -> bool {
    LEX_TOKSTR.with_borrow(|t| t.as_deref() == Some(s))
}
/// `enum lextok tok` accessors — direct port of lex.c:180 file-static.
pub fn tok() -> lextok {
    LEX_TOK.get()
}
pub fn set_tok(v: lextok) {
    LEX_TOK.set(v);
}
pub fn pos() -> usize {
    LEX_POS.get()
}
pub fn set_pos(v: usize) {
    LEX_POS.set(v);
}
/// Slice the input source from `start..end` — used by parse.rs to
/// capture function body source text. Returns None if out-of-range.
pub fn input_slice(start: usize, end: usize) -> Option<String> {
    LEX_INPUT.with_borrow(|s| s.get(start..end).map(String::from))
}

/// Create a new lexer for the given input
pub fn lex_init(input: &str) {
    // Ensure `typtab[]` is initialised so `iblank()` / `inblank()` /
    // `idigit()` / etc. (called throughout the lexer) work. C zsh
    // calls `inittyptab()` once at shell startup in `init_main()`
    // (init.c:1287); zshrs lex tests bypass that path so we kick
    // it here too. `inittyptab` is idempotent (sets `ZTF_INIT`).
    crate::ported::utils::inittyptab();
    // Reset migrated thread-locals so a fresh lexer instance
    // starts from a clean slate (same as the C source's
    // file-static initializers in lex.c).
    LEX_UNGET_BUF.with_borrow_mut(|b| b.clear());
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
    LEX_INPUT.with_borrow_mut(|s| {
        s.clear();
        s.push_str(input);
    });
    LEX_POS.set(0);
}

/// Append a char to the raw-input capture buffer. Direct port of
/// zsh/Src/lex.c:2025 `zshlex_raw_add`. Called from hgetc
/// when `lex_add_raw` is nonzero so cmd-sub bodies (`$(...)`,
/// `<(...)`, `>(...)`) can be replayed verbatim without re-lexing.
pub fn zshlex_raw_add(c: char) {
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
///   5. Stringg tokens: try checkalias, then reservation lookup
///      (lex.c:1993-2015).
///   6. Clear inalmore (lex.c:2016).
///
/// Direct port of `exalias(void)` at `Src/lex.c:1953`. No
/// parameters — reads global `aliastab`/`sufaliastab`/`reswdtab`
/// directly, mirroring C.
pub fn exalias() -> bool {
    // lex.c:1957 — `hwend()` ends the history-word region. zshrs's
    // history layer doesn't track per-word boundaries here; no-op.

    // c:1958-1962 — full faithful gate:
    //   if (interact && isset(SHINSTDIN) && !strin && incasepat <= 0
    //    && tok == STRING && !nocorrect && !(inbufflags & INP_ALIAS)
    //    && !hist_is_in_word()
    //    && (isset(CORRECTALL) || (isset(CORRECT) && incmdpos)))
    //       spckword(&tokstr, 1, incmdpos, 1);
    let inbufflags_alias =
        (crate::ported::input::inbufflags.with(|f| f.get()) & crate::ported::zsh_h::INP_ALIAS) != 0;
    let strin_set = crate::ported::input::strin.with(|c| c.get()) != 0;
    if crate::ported::zsh_h::interact()
        && isset(SHINSTDIN)
        && !strin_set
        && LEX_INCASEPAT.get() <= 0
        && tok() == STRING_LEX
        && LEX_NOCORRECT.get() == 0
        && !inbufflags_alias
        && crate::ported::hist::hist_is_in_word() == 0
        && (isset(CORRECTALL) || (isset(CORRECT) && LEX_INCMDPOS.get()))
    {
        // Build candidate list: command names in $PATH, shell
        // functions, aliases, builtins. C's spckword scans these
        // internally; our Rust spckword (utils.rs:1802) takes a
        // pre-built list, so we assemble here, run, then mutate
        // tokstr in place when a correction crosses the distance
        // threshold (matches C's `spckword(&tokstr, 1, incmdpos, 1);`
        // contract — replacing tokstr with the corrected form).
        let candidates: Vec<String> = {
            let mut v: Vec<String> = Vec::new();
            // Shell functions.
            if let Ok(t) = crate::ported::hashtable::shfunctab_lock().read() {
                v.extend(t.iter().map(|(k, _)| k.clone()));
            }
            // Aliases.
            if let Ok(t) = crate::ported::hashtable::aliastab_lock().read() {
                v.extend(t.iter().map(|(k, _)| k.clone()));
            }
            // Command names (from cmdnamtab — hashed $PATH lookups).
            if let Ok(t) = crate::ported::hashtable::cmdnamtab_lock().read() {
                v.extend(t.iter().map(|(k, _)| k.clone()));
            }
            v
        };
        let refs: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
        // C calls `spckword(&tokstr, ...)` with the raw tokstr; this
        // runs BEFORE alias / reswd lookup below, BEFORE `lextext`
        // is computed. Read `tokstr` directly here.
        if let Some(word) = tokstr() {
            let word_untok = if has_token(&word) {
                untokenize(&word)
            } else {
                word.clone()
            };
            if let Some(corrected) = crate::ported::utils::spckword(&word_untok, &refs, 3) {
                if corrected != word_untok {
                    // c:1962 spckword `doerr=1` arg → prompts user.
                    // Without per-call interactive integration, accept
                    // the correction silently when CORRECTALL is set,
                    // skip otherwise (matches the conservative path).
                    if isset(CORRECTALL) {
                        set_tokstr(Some(corrected));
                    }
                }
            }
        }
    }

    // lex.c:1964-1969 — bare-token path (no tokstr).
    if tokstr_is_none() {
        // lex.c:1965 — `zshlextext = tokstrings[tok];` — for tokens
        // like SEMI/AMPER/etc. the canonical text comes from a
        // static table.
        if tok() == NEWLIN {
            return false;
        }
        // Use punctuation-token text; unknown tokens skip alias.
        let text = match tok() {
            SEMI => ";",
            AMPER => "&",
            BAR_TOK => "|",
            _ => return false,
        };
        return checkalias(text);
    }

    let tokstr = tokstr().unwrap();
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
        gotword();
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

    // lex.c:1993-2015 — Stringg-token alias / reswd check.
    if tok() == STRING_LEX {
        // c:1995 — `if ((zshlextext != copy || !isset(POSIXALIASES))
        //   && checkalias())` — under POSIX_ALIASES, only run
        // alias expansion on tokens that came in literal (`zshlextext
        // == copy` means no tokenisation/untokenisation happened).
        // zshrs always passes the untokenised `lextext`; if the
        // original tokstr had tokens AND POSIXALIASES is on, skip
        // the alias check (matches C `zshlextext != copy`).
        let had_tokens = has_token(&tokstr);
        if (!had_tokens || !isset(POSIXALIASES)) && checkalias(&lextext) {
            return true;
        }

        // c:2002 — reserved-word lookup. Fires when in command
        // position OR when the text is bare `}` and IGNOREBRACES
        // / IGNORECLOSEBRACES are both unset (so `}` ends a brace
        // block).
        let is_close_brace_special =
            lextext == "}" && unset(IGNOREBRACES) && unset(IGNORECLOSEBRACES);
        if LEX_INCMDPOS.get() || is_close_brace_special {
            // lex.c:2002 — `(rw = (Reswd) reswdtab->getnode(reswdtab, tokstr))`
            let rw_tok: Option<lextok> = {
                let guard = crate::ported::hashtable::reswdtab_lock()
                    .read()
                    .expect("reswdtab poisoned");
                guard.get(&lextext).map(|r| r.token)
            };
            if let Some(rwtok) = rw_tok {
                set_tok(rwtok);
                if rwtok == REPEAT {
                    LEX_INREPEAT.set(1);
                }
                if rwtok == DINBRACK {
                    LEX_INCOND.set(1);
                }
            }
        } else if LEX_INCOND.get() > 0 && lextext == "]]" {
            // lex.c:2010-2012 — `]]` closes the cond expression.
            set_tok(DOUTBRACK);
            LEX_INCOND.set(0);
        } else if LEX_INCOND.get() == 1 && lextext == "!" {
            // lex.c:2013-2014 — `!` inside `[[ ]]` is the Bang
            // negation, not a literal.
            set_tok(BANG_TOK);
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
/// WARNING: param names don't match C — Rust=(lextext) vs C=()
fn checkalias(lextext: &str) -> bool {
    // lex.c:1906-1907 — guard on null lextext.
    if lextext.is_empty() {
        return false;
    }

    // c:1909 — `if (!noaliases && isset(ALIASESOPT) &&
    //   (!isset(POSIXALIASES) ||
    //    (tok == STRING && !reswdtab->getnode(reswdtab, zshlextext))))`.
    //
    // Three gates: (1) lexer hasn't set noaliases; (2) ALIASESOPT
    // option is on; (3) under POSIX_ALIASES, the token must be a
    // STRING AND not a reserved word.
    if LEX_NOALIASES.get() || !isset(ALIASESOPT) {
        return false;
    }
    if isset(POSIXALIASES) {
        if tok() != STRING_LEX {
            return false;
        }
        let is_reswd = crate::ported::hashtable::reswdtab_lock()
            .read()
            .expect("reswdtab poisoned")
            .get(lextext)
            .is_some();
        if is_reswd {
            return false;
        }
    }

    // lex.c:1914-1933 — regular alias lookup. C: `an = (Alias)
    // aliastab->getnode(aliastab, zshlextext);`
    let alias_clone: Option<crate::ported::zsh_h::alias> = {
        let guard = crate::ported::hashtable::aliastab_lock()
            .read()
            .expect("aliastab poisoned");
        guard.get(lextext).cloned()
    };
    if let Some(alias) = alias_clone {
        let is_global = (alias.node.flags & crate::ported::zsh_h::ALIAS_GLOBAL) != 0;
        if alias.inuse == 0 && (is_global || (LEX_INCMDPOS.get() && tok() == STRING_LEX)) {
            // c:1918-1927 — if the next char isn't blank, insert a
            // space so the alias body can't accidentally join the
            // following word.
            if !LEX_LEXSTOP.get() {
                if let Some(c) = peek() {
                    if !crate::ztype_h::iblank(c as u8) {
                        crate::ported::input::inpush(" ", crate::ported::zsh_h::INP_ALIAS, None);
                    }
                }
            }
            // c:1928 — `inpush(an->text, INP_ALIAS, an);`
            crate::ported::input::inpush(
                &alias.text,
                crate::ported::zsh_h::INP_ALIAS,
                Some(lextext.to_string()),
            );
            // c:1929 — `an->inuse = 1;`.
            let mut guard = crate::ported::hashtable::aliastab_lock()
                .write()
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
                    let guard = crate::ported::hashtable::sufaliastab_lock()
                        .read()
                        .expect("sufaliastab poisoned");
                    guard.get(suffix).cloned()
                };
                if let Some(alias) = alias_clone {
                    if alias.inuse == 0 {
                        // c:1938-1940 — three inpush calls in order:
                        // the original word, a space, the alias text.
                        // inpush stacks LIFO so the original word is
                        // popped FIRST (re-emitted to extend the
                        // current token), then space, then the alias
                        // body. C does it the same way.
                        crate::ported::input::inpush(
                            lextext,
                            crate::ported::zsh_h::INP_ALIAS,
                            Some(suffix.to_string()),
                        );
                        crate::ported::input::inpush(" ", crate::ported::zsh_h::INP_ALIAS, None);
                        crate::ported::input::inpush(
                            &alias.text,
                            crate::ported::zsh_h::INP_ALIAS,
                            None,
                        );
                        // c:1941 — `an->inuse = 1;`.
                        let mut guard = crate::ported::hashtable::sufaliastab_lock()
                            .write()
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

/// Pop the last char from the raw-input capture buffer. Direct
/// port of zsh/Src/lex.c:2043 `zshlex_raw_back`. Called when
/// the lexer ungets a char that was just captured raw — the raw
/// buffer must mirror the live input so this undoes the last add.
pub fn zshlex_raw_back() {
    // lex.c:2045-2046 — guard.
    if LEX_LEX_ADD_RAW.get() == 0 {
        return;
    }
    // lex.c:2047-2048 — `lexbuf_raw.ptr--; lexbuf_raw.len--;`
    LEX_LEXBUF_RAW.with_borrow_mut(|b| b.pop());
}

/// Mark the current raw-buffer offset (for restore later). Direct
/// port of zsh/Src/lex.c:2053 `zshlex_raw_mark`. Returns
/// `len + offset` so callers can restore via `back_to_mark`.
pub fn zshlex_raw_mark(offset: i64) -> i64 {
    // lex.c:2055-2056 — guard.
    if LEX_LEX_ADD_RAW.get() == 0 {
        return 0;
    }
    // lex.c:2057 — `return lexbuf_raw.len + offset;`
    (LEX_LEXBUF_RAW.with_borrow(|b| b.buf_len()) as i64) + offset
}

/// Restore raw-buffer offset to a previously-saved mark. Direct
/// port of zsh/Src/lex.c:2062 `zshlex_raw_back_to_mark`.
/// Truncates the raw buffer to `mark` bytes — undoes any captures
/// since the mark was taken (used when a speculative parse fails
/// and the lexer rolls back).
pub fn zshlex_raw_back_to_mark(mark: i64) {
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

/// zsh/Src/lex.c:216 `lex_context_save`. After save, the lexer
/// is in a clean state suitable for parsing a nested input (command
/// substitution body, here-doc terminator, eval'd string).
pub fn lex_context_save(ls: &mut lex_stack) {
    // c:218-239 — copy live state into the stack. Mirrors C
    // field-by-field; `toplevel` param dropped because C does
    // `(void)toplevel;` (unused).
    ls.dbparens = LEX_DBPARENS.get() as i32;
    ls.isfirstln = LEX_ISFIRSTLN.get() as i32;
    ls.isfirstch = LEX_ISFIRSTCH.get() as i32;
    ls.lexflags = LEX_LEXFLAGS.get();
    ls.tok = tok();
    ls.tokstr = tokstr_take();
    // `zshlextext` (c:225) — pointer alias of `tokstr` after
    // untokenization. zshrs derives it on demand from `tokstr` +
    // `untokenize` so there's no separate global to stash.
    ls.zshlextext = None;
    LEX_LEXBUF.with_borrow_mut(|b| {
        ls.lexbuf.ptr = b.ptr.take();
        ls.lexbuf.siz = b.siz;
        ls.lexbuf.len = b.len;
    });
    ls.lex_add_raw = LEX_LEX_ADD_RAW.get();
    ls.tokstr_raw = LEX_TOKSTR_RAW.with_borrow_mut(|t| t.take());
    LEX_LEXBUF_RAW.with_borrow_mut(|b| {
        ls.lexbuf_raw.ptr = b.ptr.take();
        ls.lexbuf_raw.siz = b.siz;
        ls.lexbuf_raw.len = b.len;
    });
    ls.lexstop = LEX_LEXSTOP.get() as i32;
    ls.toklineno = LEX_TOKLINENO.get() as i64;

    // c:235-238 — reset live state to defaults so a nested parse
    // starts from a clean slate. tokstr/lexbuf zeroed; lexbuf.siz
    // reset to 256 (the C-source initial alloc); raw buffers
    // wiped, lex_add_raw cleared.
    set_tokstr(None);
    LEX_LEXBUF.with_borrow_mut(|b| {
        b.ptr = Some(String::with_capacity(256));
        b.siz = 256;
        b.len = 0;
    });
    LEX_TOKSTR_RAW.with_borrow_mut(|t| *t = None);
    LEX_LEXBUF_RAW.with_borrow_mut(|b| {
        b.ptr = None;
        b.siz = 0;
        b.len = 0;
    });
    LEX_LEX_ADD_RAW.set(0);
}

/// zsh/Src/lex.c:245 `lex_context_restore`. Inverse of
/// `lex_context_save`. Called after the nested parse completes.
pub fn lex_context_restore(ls: &mut lex_stack) {
    // c:249-261 — copy stack state back into live fields.
    LEX_DBPARENS.set(ls.dbparens != 0);
    LEX_ISFIRSTLN.set(ls.isfirstln != 0);
    LEX_ISFIRSTCH.set(ls.isfirstch != 0);
    LEX_LEXFLAGS.set(ls.lexflags);
    set_tok(ls.tok);
    set_tokstr(ls.tokstr.take());
    // ls.zshlextext discarded — derived from tokstr (see save).
    let _ = ls.zshlextext.take();
    LEX_LEXBUF.with_borrow_mut(|b| {
        b.ptr = Some(ls.lexbuf.ptr.take().unwrap_or_default());
        b.siz = ls.lexbuf.siz;
        b.len = ls.lexbuf.len;
    });
    LEX_LEX_ADD_RAW.set(ls.lex_add_raw);
    LEX_TOKSTR_RAW.with_borrow_mut(|t| *t = ls.tokstr_raw.take());
    LEX_LEXBUF_RAW.with_borrow_mut(|b| {
        b.ptr = ls.lexbuf_raw.ptr.take();
        b.siz = ls.lexbuf_raw.siz;
        b.len = ls.lexbuf_raw.len;
    });
    LEX_LEXSTOP.set(ls.lexstop != 0);
    LEX_TOKLINENO.set(ls.toklineno as u64);
}

/// Initialize lexical state. Direct port of zsh/Src/lex.c:441
/// `lexinit`. Resets dbparens / nocorrect / lexstop and sets `tok`
/// to ENDINPUT so the next gettok starts from a known baseline.
/// Note: `lex_init(input)` already sets equivalent defaults; this
/// function exists for the rare case a caller wants to reset the
/// lexer state mid-parse without re-loading input.
pub fn lexinit() {
    // lex.c:443 — `nocorrect = dbparens = lexstop = 0;`
    LEX_NOCORRECT.set(0);
    LEX_DBPARENS.set(false);
    LEX_LEXSTOP.set(false);
    // lex.c:444 — `tok = ENDINPUT;`
    set_tok(ENDINPUT);
}

/// Check recursion depth; returns true if exceeded
#[inline]
/// Get next character from input
fn hgetc() -> Option<char> {
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
        // c:input.c:360-361 — every char returned by ingetc feeds
        // the raw buffer when lex_add_raw is on. Re-reads from the
        // unget queue count the same as fresh reads; the matching
        // `zshlex_raw_back()` call in hungetc removed the prior
        // record, so this restores it.
        zshlex_raw_add(c);
        return Some(c);
    }

    let c = LEX_INPUT.with_borrow(|s| s[LEX_POS.get()..].chars().next())?;
    LEX_POS.set(LEX_POS.get() + c.len_utf8());

    if c == '\n' {
        LEX_LINENO.set(LEX_LINENO.get() + 1);
    }

    // c:input.c:360-361 — `if (!lexstop) zshlex_raw_add(lastc);`
    // Every char read from input also feeds the raw buffer when
    // lex_add_raw is on (used by skipcomm to capture verbatim
    // `$(...)` body text into the parent token).
    zshlex_raw_add(c);

    Some(c)
}

/// Put character back into input
fn hungetc(c: char) {
    LEX_UNGET_BUF.with_borrow_mut(|b| b.push_front(c));
    if c == '\n' && LEX_LINENO.get() > 1 {
        LEX_LINENO.set(LEX_LINENO.get() - 1);
    }
    LEX_LEXSTOP.set(false);
    // c:input.c:549,609 — `inungetc` calls `zshlex_raw_back()` so
    // the un-gotten char isn't double-counted in lexbuf_raw on
    // re-read. hgetc will re-add it next time it's pulled.
    zshlex_raw_back();
}

/// Peek at next character without consuming
#[allow(dead_code)]
fn peek() -> Option<char> {
    if let Some(c) = LEX_UNGET_BUF.with_borrow(|b| b.front().copied()) {
        return Some(c);
    }
    LEX_INPUT.with_borrow(|s| s[LEX_POS.get()..].chars().next())
}

/// Add character to token buffer
/// Port of `add(int c)` from `Src/lex.c:451`.
fn add(c: char) {
    LEX_LEXBUF.with_borrow_mut(|b| b.add(c));
}



/// Main lexer entry point — fetch the next token. Direct port of
/// zsh/Src/lex.c:266 `zshlex`. Loop body matches the C source
/// `do { ... } while (tok != ENDINPUT && exalias())` at lex.c:270-276,
/// followed by here-doc draining (lex.c:278-306), newline tracking
/// (lex.c:307-310), and SEMI/NEWLIN→SEPER folding (lex.c:311-312).
pub fn zshlex() {
    // lex.c:268-269 — early-out on prior LEXERR.
    if tok() == LEXERR {
        return;
    }

    // lex.c:270-276 — `do { ... } while (tok != ENDINPUT && exalias())`.
    // The do-while re-runs gettok when exalias re-injects alias text;
    // exalias also performs reswdtab keyword promotion (`{` → INBRACE,
    // `if` → IF, etc.) and spell-correction. Wired one-pass for now —
    // alias re-injection loop is a follow-up.
    loop {
        // lex.c:271-272 — bump inrepeat counter for `repeat N {}`
        // detection.
        if LEX_INREPEAT.get() > 0 {
            LEX_INREPEAT.set(LEX_INREPEAT.get() + 1);
        }
        // lex.c:273-274 — `if (inrepeat_ == 3 && (isset(SHORTLOOPS) ||
        // isset(SHORTREPEAT))) incmdpos = 1;` — at the third token after
        // `repeat`, SHORTLOOPS/SHORTREPEAT options force back into
        // command position so the loop body can start without an
        // explicit `{`.
        if LEX_INREPEAT.get() == 3 && (isset(SHORTLOOPS) || isset(SHORTREPEAT)) {
            LEX_INCMDPOS.set(true);
        }

        // lex.c:275 — `tok = gettok();`
        let _t = gettok();
        set_tok(_t);

        // lex.c:276 — `} while (tok != ENDINPUT && exalias());`
        if tok() == ENDINPUT || !exalias() {
            break;
        }
    }

    // lex.c:277 — `nocorrect &= 1;` — clear bit 1 (lookahead-only)
    // so the persistent low bit survives but the per-word bit is
    // dropped.
    LEX_NOCORRECT.set(LEX_NOCORRECT.get() & 1);

    // lex.c:278-306 — drain pending here-documents at the start
    // of a new line. zshrs's process_heredocs reads the full body
    // and stitches it onto the matching redir token.
    if tok() == NEWLIN || tok() == ENDINPUT {
        process_heredocs();
    }

    // lex.c:307-310 — track whether we just saw a newline.
    // C uses `inbufct` to distinguish "newline at EOF" (=1)
    // from "newline mid-input" (=-1); zshrs reads `pos < len`.
    if tok() != NEWLIN {
        LEX_ISNEWLIN.set(0);
    } else {
        LEX_ISNEWLIN.set(if LEX_POS.get() < LEX_INPUT.with_borrow(|s| s.len()) {
            -1
        } else {
            1
        });
    }

    // lex.c:311-312 — fold SEMI / NEWLIN into SEPER unless
    // LEXFLAGS_NEWLINE is set to preserve newlines (used by
    // ZLE for completion of partial lines).
    if tok() == SEMI || (tok() == NEWLIN && LEX_LEXFLAGS.get() & LEXFLAGS_NEWLINE == 0) {
        set_tok(SEPER);
    }

    // C zshlex (lex.c:266-310) does NOT update incmdpos / inredir /
    // oldpos / infor / intypeset / incondpat. Those updates live in:
    //   - ctxtlex (lex.c:319-368, mirrored at fn ctxtlex below) for
    //     incmdpos / inredir / oldpos / infor;
    //   - parse.c call sites for intypeset (parse.c:1932/2042/2047);
    //   - cond.c par_cond_* for incondpat (Rust-only state — C tracks
    //     pattern context implicitly via the cond grammar walker).
    // Earlier zshrs port had duplicated the ctxtlex switch + an
    // incondpat tracker into zshlex so the parser would get those
    // updates "for free"; that broke the C-faithful contract of
    // zshlex. Removed.
}

/// Process pending here-documents. Walks each heredoc whose body
/// hasn't been filled yet (content is empty AND terminator is set),
/// reads lines from input until the terminator, and stuffs the body
/// into `hdoc.content` IN PLACE. The list itself is preserved so the
/// parser can index into it after parse() finishes.
fn process_heredocs() {
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
        // c:284 — `cmdpush(hdocs->type == REDIR_HEREDOC ? CS_HEREDOC :
        // CS_HEREDOCD);` — `<<-` (strip_tabs) is CS_HEREDOCD; bare
        // `<<` is CS_HEREDOC.
        cmdpush(if strip_tabs {
            CS_HEREDOCD as u8
        } else {
            CS_HEREDOC as u8
        });
        let mut content = String::new();

        loop {
            let line = read_line();
            if line.is_none() {
                // c:292 — `zerr("here document too large");` then
                // tok = LEXERR + cmdpop + bail out of the heredoc loop.
                crate::ported::utils::zerr("here document too large");
                set_tok(LEXERR);
                cmdpop();
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

        // c:289 — `cmdpop();` matches c:284 push on normal completion.
        cmdpop();

        LEX_HEREDOCS.with_borrow_mut(|v| {
            v[i].content = content;
            v[i].processed = true;
        });
    }
}

/// Read a line from input (returns partial line at EOF)
fn read_line() -> Option<String> {
    let mut line = String::new();

    loop {
        match hgetc() {
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

// `hashchar` / `bangchar` / `hatchar` — port of `unsigned char
// hashchar, bangchar, hatchar;` declared in `Src/params.c:132` and
// assigned in `init_main()` (Src/init.c:1100-1102). C zsh exposes
// these as mutable globals for `$histchars` to rewrite; zshrs
// pins the default values here (the `$histchars` runtime mirror
// lives at `crate::ported::hist::bangchar` (AtomicI32)).
#[allow(non_upper_case_globals)]
const hashchar: char = '#';
#[allow(non_upper_case_globals)]
const bangchar: char = '!';
#[allow(non_upper_case_globals)]
#[allow(dead_code)]
const hatchar: char = '^';

/// Get the next token. Direct port of `gettok(void)` from
/// `Src/lex.c:613-936`. Reads chars via hgetc, dispatches on the
/// leading char through the `lexact1[]` table at c:725. The 23
/// LX1_* arms below mirror C's `switch (lexact1[STOUC(c)])` body
/// one-for-one; the LX1_OTHER fallthrough at c:918 routes to
/// `gettokstr` for word-shape lexing.
fn gettok() -> lextok {
    // c:617 — `int peekfd = -1;`. Local fd-prefix accumulator.
    // Written into the global `tokfd` ONLY at redir-arm exit
    // (c:753, 864, 913) — mirrored in lex_inang / lex_outang.
    let mut peekfd: i32 = -1;
    // c:620 — `tokstr = NULL;`. Reset before each token.
    set_tokstr(None);

    // c:622 — `while (iblank(c = hgetc()) && !lexstop);` — skip
    // leading blanks (space/tab, NOT newline). Keep `c` from the
    // loop; don't unget+reread.
    let c = loop {
        match hgetc() {
            Some(ch) if crate::ztype_h::iblank(ch as u8) => continue,
            Some(ch) => break ch,
            None => {
                // c:624-625 — `if (lexstop) return (errflag) ?
                // LEXERR : ENDINPUT;`
                use std::sync::atomic::Ordering;
                LEX_LEXSTOP.set(true);
                return if crate::ported::utils::errflag.load(Ordering::Relaxed) != 0 {
                    LEXERR
                } else {
                    ENDINPUT
                };
            }
        }
    };

    // c:623 — `toklineno = lineno;` runs BEFORE the lexstop check
    // (matches C ordering: assign first, then check stop).
    LEX_TOKLINENO.set(LEX_LINENO.get());
    // c:626 — `isfirstln = 0;`
    LEX_ISFIRSTLN.set(false);

    // c:627-628 — `if ((lexflags & LEXFLAGS_ZLE) && !(inbufflags
    // & INP_ALIAS)) wordbeg = inbufct - (qbang && c == bangchar);`
    // ZLE word-begin tracking; consumed by `gotword` (c:1882).
    let qbang_at_bang =
        crate::ported::hist::qbang.load(std::sync::atomic::Ordering::SeqCst) && c == bangchar;
    let qbang_adj: i32 = if qbang_at_bang { 1 } else { 0 };
    if (LEX_LEXFLAGS.get() & LEXFLAGS_ZLE) != 0
        && (crate::ported::input::inbufflags.with(|f| f.get()) & crate::ported::zsh_h::INP_ALIAS)
            == 0
    {
        LEX_WORDBEG.set(crate::ported::input::inbufct.with(|c| c.get()) - qbang_adj);
    }
    // c:630 — `hwbegin(-1-(qbang && c == bangchar));` — start a
    // new history word. C `hwbegin` is a function pointer flipped
    // by `hbegin()` between `ihwbegin` (real, hist.c:1656) and
    // `nohwb` (no-op). zshrs doesn't flip the pointer yet but
    // `ihwbegin` itself is a no-op when history is inactive
    // (`stophist == 2` or `histactive & HA_INWORD`), so calling it
    // unconditionally matches the inactive case.
    crate::ported::hist::ihwbegin(-1 - qbang_adj);

    // c:631-648 — `if (dbparens)` block, inlined verbatim from
    // gettok body. Lexes the body of `(( ... ))` arithmetic.
    if LEX_DBPARENS.get() {
        // c:632 — `lexbuf.len = 0; lexbuf.ptr = tokstr = hcalloc(...);`
        LEX_LEXBUF.with_borrow_mut(|b| b.clear());
        // c:633 — `hungetc(c);`
        hungetc(c);
        // c:634-637 — `cmdpush(CS_MATH); c = dquote_parse(infor ?
        // ';' : ')', 0); cmdpop();`
        let end_char = if LEX_INFOR.get() > 0 { ';' } else { ')' };
        cmdpush(CS_MATH as u8);
        let parse_ok = dquote_parse(end_char, false).is_ok();
        cmdpop();
        if !parse_ok {
            return LEXERR;
        }
        // c:638 — `*lexbuf.ptr = '\0';`
        set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));
        // c:639-642 — `if (!c && infor) { infor--; return DINPAR; }`
        if LEX_INFOR.get() > 0 {
            LEX_INFOR.set(LEX_INFOR.get() - 1);
            return DINPAR;
        }
        // c:643-646 — `if (c || (c = hgetc()) != ')') { hungetc(c);
        // return LEXERR; }`
        match hgetc() {
            Some(')') => {
                // c:647 — `dbparens = 0;` / c:648 — `return DOUTPAR;`
                LEX_DBPARENS.set(false);
                return DOUTPAR;
            }
            c => {
                if let Some(c) = c {
                    hungetc(c);
                }
                return LEXERR;
            }
        }
    }

    // treats `2` as the fd to redirect. Three shapes: `N>`/`N<`
    // (single redir), `N&>` (errwrite), or anything else (push
    // back, treat as literal digit). The digit is captured into
    // `peekfd` and `c` is rewritten to the operator char so the
    // LX1 dispatch sees the redir, not the digit.
    let mut c = c;
    if crate::ztype_h::idigit(c as u8) {
        let d = hgetc();
        match d {
            Some('&') => {
                let e = hgetc();
                if e == Some('>') {
                    // c:653-657 — `N&>` shape detected.
                    peekfd = (c as u8 - b'0') as i32;
                    hungetc('>');
                    c = '&';
                } else {
                    // c:658-661 — not `N&>`, push everything back.
                    if let Some(e) = e {
                        hungetc(e);
                    }
                    hungetc('&');
                }
            }
            Some('>') | Some('<') => {
                // c:662-664 — `N>` or `N<` shape detected.
                peekfd = (c as u8 - b'0') as i32;
                c = d.unwrap();
            }
            Some(d) => {
                // c:665-668 — not a redir prefix, push back.
                hungetc(d);
            }
            None => {}
        }
        LEX_LEXSTOP.set(false);
    }

    // c:678 — `if (c == hashchar && !nocomments &&
    //   (isset(INTERACTIVECOMMENTS) ||
    //    ((!lexflags || (lexflags & LEXFLAGS_COMMENTS)) && !expanding &&
    //     (!interact || unset(SHINSTDIN) || strin))))`.
    //
    // Comments only fire when `#` is at word-start AND one of:
    //   1. INTERACTIVECOMMENTS option is set; OR
    //   2. Non-interactive: lexflags allows comments AND not currently
    //      expanding AND (not interactive OR not reading stdin OR
    //      reading from a string).
    //
    // `expanding` and `strin` globals aren't ported yet — treated as 0
    // (the safe default for non-completion non-string-eval paths).
    let lexflags = LEX_LEXFLAGS.get();
    let allow_comment_via_flags = (lexflags == 0 || (lexflags & LEXFLAGS_COMMENTS) != 0)
        && (!crate::ported::zsh_h::interact() || unset(SHINSTDIN));
    if c == hashchar
        && !LEX_NOCOMMENTS.get()
        && (isset(INTERACTIVECOMMENTS) || allow_comment_via_flags)
    {
        // c:686-707 — comment body. Under LEXFLAGS_COMMENTS_KEEP,
        // capture the comment text as a STRING token; under
        // LEXFLAGS_COMMENTS_STRIP, return ENDINPUT at EOF; default
        // is to read-and-drop the comment and return NEWLIN.
        if LEX_LEXFLAGS.get() & LEXFLAGS_COMMENTS_KEEP != 0 {
            LEX_LEXBUF.with_borrow_mut(|b| b.clear());
            add('#');
        }
        loop {
            let c = hgetc();
            match c {
                Some('\n') | None => break,
                Some(c) => {
                    if LEX_LEXFLAGS.get() & LEXFLAGS_COMMENTS_KEEP != 0 {
                        add(c);
                    }
                }
            }
        }
        if LEX_LEXFLAGS.get() & LEXFLAGS_COMMENTS_KEEP != 0 {
            set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));
            if !LEX_LEXSTOP.get() {
                hungetc('\n');
            }
            return STRING_LEX;
        }
        if LEX_LEXFLAGS.get() & LEXFLAGS_COMMENTS_STRIP != 0 && LEX_LEXSTOP.get() {
            return ENDINPUT;
        }
        return NEWLIN;
    }

    // c:725 — `switch (lexact1[STOUC(c)])` table-driven dispatch.
    let act = lexact1_get(c);
    match act {
        LX1_BKSLASH => {
            let d = hgetc();
            if d == Some('\n') {
                // Line continuation - get next token
                return gettok();
            }
            if let Some(d) = d {
                hungetc(d);
            }
            LEX_LEXSTOP.set(false);
            gettokstr(c, false)
        }

        LX1_NEWLIN => NEWLIN,

        LX1_SEMI => {
            let d = hgetc();
            match d {
                Some(';') => DSEMI,
                Some('&') => SEMIAMP,
                Some('|') => SEMIBAR,
                _ => {
                    if let Some(d) = d {
                        hungetc(d);
                    }
                    LEX_LEXSTOP.set(false);
                    SEMI
                }
            }
        }

        LX1_AMPER => {
            let d = hgetc();
            match d {
                Some('&') => DAMPER,
                Some('!') | Some('|') => AMPERBANG,
                Some('>') => {
                    // c:753 — `tokfd = peekfd;` before the `&>` shape
                    // continues into the LX1_OUTANG-like dispatch.
                    LEX_TOKFD.set(peekfd);
                    let e = hgetc();
                    match e {
                        Some('!') | Some('|') => OUTANGAMPBANG,
                        Some('>') => {
                            let f = hgetc();
                            match f {
                                Some('!') | Some('|') => DOUTANGAMPBANG,
                                _ => {
                                    if let Some(f) = f {
                                        hungetc(f);
                                    }
                                    LEX_LEXSTOP.set(false);
                                    DOUTANGAMP
                                }
                            }
                        }
                        _ => {
                            if let Some(e) = e {
                                hungetc(e);
                            }
                            LEX_LEXSTOP.set(false);
                            AMPOUTANG
                        }
                    }
                }
                _ => {
                    if let Some(d) = d {
                        hungetc(d);
                    }
                    LEX_LEXSTOP.set(false);
                    AMPER
                }
            }
        }

        LX1_BAR => {
            let d = hgetc();
            match d {
                Some('|') if LEX_INCASEPAT.get() <= 0 => DBAR,
                Some('&') => BARAMP,
                _ => {
                    if let Some(d) = d {
                        hungetc(d);
                    }
                    LEX_LEXSTOP.set(false);
                    BAR_TOK
                }
            }
        }

        LX1_INPAR => {
            let d = hgetc();
            match d {
                Some('(') => {
                    if LEX_INFOR.get() > 0 {
                        LEX_DBPARENS.set(true);
                        return DINPAR;
                    }
                    // c:788 — `if (incmdpos || (isset(SHGLOB) &&
                    // !isset(KSHGLOB)))` — under SHGLOB-without-KSHGLOB,
                    // `((` is also math/subshell-eligible even when
                    // not at command position.
                    if LEX_INCMDPOS.get() || (isset(SHGLOB) && unset(KSHGLOB)) {
                        // Could be (( arithmetic )) or ( subshell )
                        LEX_LEXBUF.with_borrow_mut(|b| b.clear());
                        match cmd_or_math() {
                            CMD_OR_MATH_MATH => {
                                set_tokstr(Some(
                                    LEX_LEXBUF.with_borrow(|b| b.as_str().to_string()),
                                ));
                                return DINPAR;
                            }
                            CMD_OR_MATH_CMD => {
                                set_tokstr(None);
                                return INPAR_TOK;
                            }
                            CMD_OR_MATH_ERR | _ => return LEXERR,
                        }
                    }
                    hungetc('(');
                    LEX_LEXSTOP.set(false);
                    gettokstr('(', false)
                }
                Some(')') => INOUTPAR,
                _ => {
                    if let Some(d) = d {
                        hungetc(d);
                    }
                    LEX_LEXSTOP.set(false);
                    // c:821 — `if (!(isset(SHGLOB) || incond == 1 ||
                    // incmdpos)) break; return INPAR;` — at word
                    // boundary `(` tokenizes as Inpar when SHGLOB or
                    // incond==1 or incmdpos. Otherwise breaks out
                    // (falls through to gettokstr so `(` starts a
                    // Stringg — typical for unquoted glob args like
                    // `ls (^foo)*`).
                    if isset(SHGLOB)
                        || LEX_INCOND.get() == 1
                        || LEX_INCMDPOS.get()
                        || LEX_INCASEPAT.get() >= 1
                    {
                        INPAR_TOK
                    } else {
                        gettokstr('(', false)
                    }
                }
            }
        }

        LX1_OUTPAR => OUTPAR_TOK,

        // c:826-864 — `case LX1_INANG:` body. In pattern context
        // (`incondpat`/`incasepat`), `<` is literal and falls
        // through to gettokstr (zshrs-only guard for `[[ < ]]`).
        LX1_INANG => {
            if LEX_INCONDPAT.get() || LEX_INCASEPAT.get() > 0 {
                return gettokstr(c, false);
            }
            let d = hgetc();
            let peek = match d {
                Some('(') => {
                    // c:828-832 — `<(...)` process substitution.
                    // Fall through to gettokstr; tokfd write doesn't
                    // apply (process-sub, not a redir).
                    hungetc('(');
                    LEX_LEXSTOP.set(false);
                    return gettokstr('<', false);
                }
                Some('>') => INOUTANG,
                Some('<') => {
                    let e = hgetc();
                    match e {
                        Some('(') => {
                            hungetc('(');
                            hungetc('<');
                            INANG_TOK
                        }
                        Some('<') => TRINANG,
                        Some('-') => DINANGDASH,
                        _ => {
                            if let Some(e) = e {
                                hungetc(e);
                            }
                            LEX_LEXSTOP.set(false);
                            DINANG
                        }
                    }
                }
                Some('&') => INANGAMP,
                _ => {
                    if let Some(d) = d {
                        hungetc(d);
                    }
                    LEX_LEXSTOP.set(false);
                    INANG_TOK
                }
            };
            // c:864 — `tokfd = peekfd; return peek;`.
            LEX_TOKFD.set(peekfd);
            peek
        }

        // c:866-914 — `case LX1_OUTANG:` body.
        LX1_OUTANG => {
            if LEX_INCONDPAT.get() || LEX_INCASEPAT.get() > 0 {
                return gettokstr(c, false);
            }
            let d = hgetc();
            let peek = match d {
                Some('(') => {
                    // `>(...)` process substitution.
                    hungetc('(');
                    LEX_LEXSTOP.set(false);
                    return gettokstr('>', false);
                }
                Some('&') => {
                    let e = hgetc();
                    match e {
                        Some('!') | Some('|') => OUTANGAMPBANG,
                        _ => {
                            if let Some(e) = e {
                                hungetc(e);
                            }
                            LEX_LEXSTOP.set(false);
                            OUTANGAMP
                        }
                    }
                }
                Some('!') | Some('|') => OUTANGBANG,
                Some('>') => {
                    let e = hgetc();
                    match e {
                        Some('&') => {
                            let f = hgetc();
                            match f {
                                Some('!') | Some('|') => DOUTANGAMPBANG,
                                _ => {
                                    if let Some(f) = f {
                                        hungetc(f);
                                    }
                                    LEX_LEXSTOP.set(false);
                                    DOUTANGAMP
                                }
                            }
                        }
                        Some('!') | Some('|') => DOUTANGBANG,
                        Some('(') => {
                            hungetc('(');
                            hungetc('>');
                            OUTANG_TOK
                        }
                        _ => {
                            if let Some(e) = e {
                                hungetc(e);
                            }
                            LEX_LEXSTOP.set(false);
                            // c:903 — `if (isset(HISTALLOWCLOBBER))
                            // hwaddc('|');`
                            if isset(HISTALLOWCLOBBER) {
                                hwaddc('|');
                            }
                            DOUTANG
                        }
                    }
                }
                _ => {
                    if let Some(d) = d {
                        hungetc(d);
                    }
                    LEX_LEXSTOP.set(false);
                    // c:910 — `if (!incond && isset(HISTALLOWCLOBBER))
                    // hwaddc('|');`
                    if LEX_INCOND.get() == 0 && isset(HISTALLOWCLOBBER) {
                        hwaddc('|');
                    }
                    OUTANG_TOK
                }
            };
            // c:913 — `tokfd = peekfd; return peek;`.
            LEX_TOKFD.set(peekfd);
            peek
        }

        // c:918 — `case LX1_OTHER: return gettokstr(c, 0);`. All
        // non-redir, non-separator chars (including `{`/`}`/`[`/`]`)
        // fall into gettokstr; the LX2_* arms handle word shape.
        // Reserved-word promotion (`{` → INBRACE, `[[` → DINBRACK,
        // etc.) happens in exalias via reswdtab lookup (c:2002-2005).
        _ => gettokstr(c, false),
    }
}

/// Port of `void (*hwaddc)(int)` from `Src/hist.c:43` — function-
/// pointer that the history layer flips between `ihwaddc` (real
/// append at hist.c:357, ported at `hist::ihwaddc`) when history
/// is active, and `nohw` (dummy at hist.c:1062) when it isn't.
/// Setup happens in `hbegin()` at hist.c:1141/1151. zshrs doesn't
/// flip the pointer yet, but `ihwaddc` is itself a no-op when
/// `chline` is empty, so calling it unconditionally matches C
/// behavior for the history-inactive case. The HISTALLOWCLOBBER
/// caller sites at `Src/lex.c:903, 910` write `|` so the history
/// line records the implicit clobber as `>|`/`>>|`.
#[inline]
fn hwaddc(c: char) {
    crate::ported::hist::ihwaddc(c as i32);
}

/// Get rest of token string
/// Port of `gettokstr(int c, int sub)` from `Src/lex.c:937`.
fn gettokstr(c: char, sub: bool) -> lextok {
    let mut bct = 0; // brace count
    let mut pct = 0; // parenthesis count
    let mut brct = 0; // bracket count
    let mut in_brace_param = 0;
    // c:940 — `int cmdsubst = 0;`. Tracks whether we entered the
    // brace from `$(...)` command substitution (relevant for the
    // IGNOREBRACES check at lex.c:1138).
    let mut cmdsubst: bool = false;
    let _ = &mut cmdsubst; // suppress unused-warn until LX2_STRING wires it
    let mut peek = STRING_LEX;
    let mut intpos = 1;
    let mut unmatched = '\0';
    let mut c = c;

    if !sub {
        LEX_LEXBUF.with_borrow_mut(|b| b.clear());
    }

    loop {
        let inbl = crate::ztype_h::inblank(c as u8);

        if inbl && in_brace_param == 0 && pct == 0 {
            // Whitespace outside brace param ends token
            break;
        }

        // c:954 — `switch (lexact2[STOUC(c)])` table-driven dispatch.
        // Each LX2_* arm below mirrors one C `case` from Src/lex.c.
        // Char-with-context guards (`bct > 0`, `brct > 0`, etc.) stay
        // inline where C uses them.
        match lexact2_get(c) {
            // c:982 — `case LX2_OUTPAR:` — `)`.
            //
            // c:989 — `if ((sub || in_brace_param) && isset(SHGLOB)) break;`
            // Under SHGLOB, a `)` inside `${...}` or substitution context
            // ends the token rather than being added.
            LX2_OUTPAR => {
                if (sub || in_brace_param > 0) && isset(SHGLOB) {
                    break;
                }
                if in_brace_param > 0 || sub {
                    add(Outpar);
                } else if pct > 0 {
                    pct -= 1;
                    add(Outpar);
                } else {
                    break;
                }
            }

            // c:1001 — `case LX2_BAR:`.
            //
            // c:1005 — `if (unset(SHGLOB) || (!sub && !in_brace_param))
            // c = Bar;` — under SHGLOB inside substitution/brace
            // param, `|` is left as literal `|` not tokenised to `Bar`.
            LX2_BAR => {
                if pct == 0 && in_brace_param == 0 {
                    if sub {
                        add(c);
                    } else {
                        break;
                    }
                } else if unset(SHGLOB) || (!sub && in_brace_param == 0) {
                    add(Bar);
                } else {
                    add(c);
                }
            }

            // c:1019 — `case LX2_STRING:` — `$`.
            LX2_STRING => {
                let e = hgetc();
                match e {
                    Some('\\') => {
                        let f = hgetc();
                        if f != Some('\n') {
                            if let Some(f) = f {
                                hungetc(f);
                            }
                            hungetc('\\');
                            add(Stringg);
                        } else {
                            // Line continuation after $
                            continue;
                        }
                    }
                    Some('[') => {
                        // c:1023 — `$[...]` arithmetic substitution.
                        // C: `cmdpush(CS_MATHSUBST); ... c =
                        // dquote_parse(']', sub); cmdpop();`
                        add(Stringg);
                        add(Inbrack);
                        cmdpush(CS_MATHSUBST as u8);
                        let r = dquote_parse(']', sub);
                        cmdpop();
                        if r.is_err() {
                            peek = LEXERR;
                            break;
                        }
                        add(Outbrack);
                    }
                    Some('(') => {
                        // $(...) or $((...))
                        add(Stringg);
                        match cmd_or_math_sub() {
                            CMD_OR_MATH_CMD => add(Outpar),
                            CMD_OR_MATH_MATH => add(Outparmath),
                            CMD_OR_MATH_ERR | _ => {
                                peek = LEXERR;
                                break;
                            }
                        }
                    }
                    Some('{') => {
                        // c:1053 — `${...}` parameter expansion. C:
                        // `cmdpush(CS_BRACEPAR); ++bct;
                        // if (!in_brace_param) in_brace_param = bct;`
                        add(c);
                        add(Inbrace);
                        bct += 1;
                        cmdpush(CS_BRACEPAR as u8);
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
                        add(Qstring);
                        add(Snull);
                        loop {
                            let ch = hgetc();
                            match ch {
                                Some('\'') => break,
                                Some('\\') => {
                                    let next = hgetc();
                                    match next {
                                        Some(n) => {
                                            if n == '\\' || n == '\'' {
                                                add(Bnull);
                                            } else {
                                                add('\\');
                                            }
                                            add(n);
                                        }
                                        None => {
                                            LEX_LEXSTOP.set(true);
                                            unmatched = '\'';
                                            peek = LEXERR;
                                            break;
                                        }
                                    }
                                }
                                Some(ch) => add(ch),
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
                        add(Snull);
                    }
                    Some('"') => {
                        // $"..." localized string. Same shape as a
                        // plain "..." but flagged via Qstring+Dnull
                        // so post-lex translation can substitute.
                        add(Qstring);
                        add(Dnull);
                        if dquote_parse('"', sub).is_err() {
                            peek = LEXERR;
                            break;
                        }
                        add(Dnull);
                    }
                    _ => {
                        if let Some(e) = e {
                            hungetc(e);
                        }
                        LEX_LEXSTOP.set(false);
                        add(Stringg);
                    }
                }
            }

            // c:1057 — `case LX2_INBRACK:` — `[`.
            LX2_INBRACK => {
                if in_brace_param == 0 {
                    brct += 1;
                }
                add(Inbrack);
            }

            // c:1063 — `case LX2_OUTBRACK:` — `]`.
            LX2_OUTBRACK => {
                if in_brace_param == 0 && brct > 0 {
                    brct -= 1;
                }
                add(Outbrack);
            }

            // c:1078 — `case LX2_INPAR:` — `(`.
            LX2_INPAR => {
                // c:1079-1086 — under SHGLOB, `(` inside `${...}` or
                // substitution context breaks; and `(` after a non-
                // empty lexbuf is a break unless KSHGLOB is also set
                // (KSH-style `name(...)` pattern matching).
                if isset(SHGLOB) {
                    if sub || in_brace_param > 0 {
                        break;
                    }
                    if unset(KSHGLOB) && LEX_LEXBUF.with_borrow(|b| b.len) > 0 {
                        break;
                    }
                }
                // c:1086-1135 — when `(` appears inside a Stringg and
                // is immediately followed by `)`, the string
                // terminates at the `(`. The `()` is then re-lexed as
                // a separate INOUTPAR token. This handles function
                // definitions: `name()` lexes as Stringg `name` +
                // INOUTPAR `()`, not Stringg `name()`.
                //
                // c:1112-1131 — under SHGLOB, a `(` followed by
                // whitespace at the start of a command-position word
                // (no nested brackets/braces) is a ksh function
                // definition signal — same break-out behaviour.
                if in_brace_param == 0 && !sub {
                    let e = hgetc();
                    if let Some(ch) = e {
                        hungetc(ch);
                    }
                    LEX_LEXSTOP.set(false);
                    let is_inblank = matches!(e, Some(' ' | '\t'));
                    if e == Some(')')
                        || (isset(SHGLOB)
                            && is_inblank
                            && bct == 0
                            && brct == 0
                            && intpos > 0
                            && LEX_INCMDPOS.get())
                    {
                        // `name()` (or KSH-style `name( ... )`)
                        // — terminate Stringg at `(` so the
                        // following `()` re-lexes as INOUTPAR.
                        break;
                    }
                }
                if in_brace_param == 0 {
                    pct += 1;
                }
                add(Inpar);
            }

            // c:1137 — `case LX2_INBRACE:` — `{`.
            //
            // c:1138 — `if ((isset(IGNOREBRACES) && !cmdsubst) || sub)
            // c = '{';` — with IGNOREBRACES (or in substitution
            // context), `{` is added as a literal `{`, not tokenised
            // as Inbrace, and bct is NOT incremented.
            //
            // c:1141 — `if (!lexbuf.len && incmdpos) { add('{'); ...
            // return STRING; }` — at command position with an empty
            // buffer, return immediately as STRING("{") so the post-
            // lex `reswdtab` lookup can promote it to INBRACE_TOK
            // (the `{` keyword entry in `reswdtab`).
            //
            // c:1146 — `if (in_brace_param) cmdpush(CS_BRACE); bct++;`
            // — when entering a brace inside a `${...}` (or any other
            // bct++ path), push a CS_BRACE context for prompt/completion
            // tracking. After the switch, C falls through to `add(c)`
            // at lex.c:1420, where `c` was rewritten via `lextok2[c]`
            // (lex.c:964) to the `Inbrace` marker (lex.c:429:
            // `lextok2['{'] = Inbrace`). The Rust port doesn't have the
            // pre-switch `c = lextok2[c]` rewrite OR the post-switch
            // `add(c)` — both arms must be inlined per LX2 case. The
            // earlier comment claimed C "silently swallows" `{` here;
            // that was wrong (verified at lex.c:1420 in the parent
            // gettokstr loop).
            LX2_INBRACE => {
                if (isset(IGNOREBRACES) && !cmdsubst) || sub {
                    add('{');
                } else {
                    if LEX_LEXBUF.with_borrow(|b| b.len) == 0 && LEX_INCMDPOS.get() {
                        // c:1141-1144 — `add('{'); *lexbuf.ptr = '\0'; return STRING;`
                        // Direct return; C does NOT fall through to the
                        // post-loop `hungetc(c)` because `{` was fully
                        // consumed.
                        add('{');
                        set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));
                        return STRING_LEX;
                    }
                    if in_brace_param > 0 {
                        cmdpush(CS_BRACE as u8);
                    }
                    // Track braces for both ${...} param expansion and {...} brace expansion
                    bct += 1;
                    // c:1420 — `add(c)` after switch, with c = lextok2['{']
                    // = Inbrace (c:429). Inlined here since the Rust port
                    // skipped the unconditional post-switch add.
                    add(Inbrace);
                }
            }

            // c:1152 — `case LX2_OUTBRACE:` — `}`.
            //
            // c:1153 — `if ((isset(IGNOREBRACES) || sub) && !in_brace_param)
            // break;` — under IGNOREBRACES (or substitution
            // context), and not inside a `${...}`, the `}` is added
            // as a literal `}` (via the LX2_OTHER-style fallthrough).
            //
            // c:1158-1162 — `if (in_brace_param) cmdpop(); if (bct--
            // == in_brace_param) { if (cmdsubst) cmdpop();
            // in_brace_param = cmdsubst = in_pattern = 0; }` — pop
            // the matching CS_BRACE (and CS_BRACEPAR if closing the
            // outermost ${...}).
            LX2_OUTBRACE => {
                if (isset(IGNOREBRACES) || sub) && in_brace_param == 0 {
                    add('}');
                } else if in_brace_param > 0 {
                    cmdpop(); // matches CS_BRACE or CS_BRACEPAR
                    if bct == in_brace_param {
                        if cmdsubst {
                            cmdpop();
                        }
                        in_brace_param = 0;
                        cmdsubst = false;
                    }
                    bct -= 1;
                    add(Outbrace);
                } else if bct > 0 {
                    // Closing a brace expansion like {a,b}
                    bct -= 1;
                    add(c);
                } else {
                    // c:1156 — `if (!bct) break;` breaks out of the
                    // SWITCH (not the loop), then falls through to
                    // c:1420 `add(c)`. So a stray `}` (no matching `{`)
                    // is added as a literal `}` to the lexbuf, and the
                    // post-lex `s == "}"` check at zshlex promotes it
                    // to OUTBRACE_TOK. The previous Rust port broke
                    // out of the outer loop here, dropping `}` and
                    // returning STRING("") which never promoted.
                    add(c);
                }
            }

            LX2_OUTANG => {
                // In pattern context (incondpat), > is literal
                if in_brace_param > 0 || sub || LEX_INCONDPAT.get() || LEX_INCASEPAT.get() > 0 {
                    add(c);
                } else {
                    let e = hgetc();
                    if e != Some('(') {
                        if let Some(e) = e {
                            hungetc(e);
                        }
                        LEX_LEXSTOP.set(false);
                        break;
                    }
                    // >(...)
                    add(OutangProc);
                    if skipcomm().is_err() {
                        peek = LEXERR;
                        break;
                    }
                    add(Outpar);
                }
            }

            // c:1187 — `case LX2_INANG:` — `<`.
            //
            // c:1188 — `if (isset(SHGLOB) && sub) break;` — under
            // SHGLOB inside substitution context, `<` ends the
            // token (so e.g. `$(< file)` works as input redirection).
            LX2_INANG => {
                if isset(SHGLOB) && sub {
                    break;
                }
                // In pattern context (incondpat), < is literal
                if in_brace_param > 0 || sub || LEX_INCONDPAT.get() || LEX_INCASEPAT.get() > 0 {
                    add(c);
                } else if {
                    // c:1201 — `if(isnumglob()) { add(Inang); while
                    // ((c = hgetc()) != '>') add(c); c = Outang; }`.
                    // Our `isnumglob(input, pos)` scans the static
                    // input slice rather than consuming via hgetc, so
                    // we snapshot the input from the current pos. The
                    // unget buffer (line-continuation push-backs etc.)
                    // isn't consulted here — the range glob is a
                    // word-internal shape so the unget buf is empty in
                    // practice at this site.
                    let lookahead = LEX_INPUT.with_borrow(|s| {
                        s[LEX_POS.get()..].to_string()
                    });
                    isnumglob(&lookahead, 0)
                } {
                    // c:1203-1206 — read `[0-9]*-[0-9]*>` swallow into
                    // the word, emit `<…>` as a range glob.
                    add(c);
                    while let Some(ch) = hgetc() {
                        add(ch);
                        if ch == '>' {
                            break;
                        }
                    }
                } else {
                    let e = hgetc();
                    if e != Some('(') {
                        if let Some(e) = e {
                            hungetc(e);
                        }
                        LEX_LEXSTOP.set(false);
                        break;
                    }
                    // <(...)
                    add(Inang);
                    if skipcomm().is_err() {
                        peek = LEXERR;
                        break;
                    }
                    add(Outpar);
                }
            }

            LX2_EQUALS => {
                if !sub {
                    if intpos > 0 {
                        // At start of token, check for =(...) process substitution
                        let e = hgetc();
                        if e == Some('(') {
                            add(Equals);
                            if skipcomm().is_err() {
                                peek = LEXERR;
                                break;
                            }
                            add(Outpar);
                        } else {
                            if let Some(e) = e {
                                hungetc(e);
                            }
                            LEX_LEXSTOP.set(false);
                            add(Equals);
                        }
                    } else if peek != ENVSTRING
                        && (LEX_INCMDPOS.get() || LEX_INTYPESET.get())
                        && bct == 0
                        && brct == 0
                        && LEX_INCASEPAT.get() == 0
                    {
                        // Check for VAR=value assignment (but not in case pattern context)
                        let tok_so_far = LEX_LEXBUF.with_borrow(|b| b.as_str().to_string());
                        if is_valid_assignment_target(&tok_so_far) {
                            let next = hgetc();
                            if next == Some('(') {
                                // VAR=(...) array assignment. Per zsh
                                // (lex.c emits ENVARRAY with tokstr =
                                // just the variable name, NOT
                                // including the `=`). The `=` and
                                // `(` are consumed by the lexer; the
                                // parser knows ENVARRAY means assign-
                                // array and reads the body that
                                // follows.
                                set_tokstr(Some(
                                    LEX_LEXBUF.with_borrow(|b| b.as_str().to_string()),
                                ));
                                return ENVARRAY;
                            }
                            if let Some(next) = next {
                                hungetc(next);
                            }
                            LEX_LEXSTOP.set(false);
                            peek = ENVSTRING;
                            intpos = 2;
                            add(Equals);
                        } else {
                            add(Equals);
                        }
                    } else {
                        add(Equals);
                    }
                } else {
                    add(Equals);
                }
            }

            // c:1322 — `case LX2_BKSLASH:` — `\`.
            LX2_BKSLASH => {
                let next = hgetc();
                if next == Some('\n') {
                    // Line continuation
                    let next = hgetc();
                    if let Some(next) = next {
                        c = next;
                        continue;
                    }
                    break;
                } else {
                    add(Bnull);
                    if let Some(next) = next {
                        add(next);
                    }
                }
            }

            // c:1257 — `case LX2_QUOTE:` — `'`.
            //
            // c:1307 — `else if (!sub && isset(CSHJUNKIEQUOTES) &&
            // c == '\n')` — under CSHJUNKIEQUOTES, a `\n` inside a
            // single-quoted string terminates the string (unless
            // preceded by `\`, in which case both are stripped).
            //
            // c:1328 — `e = hgetc(); if (e != '\'' || unset(RCQUOTES)
            // || strquote) break; add(c);` — under RCQUOTES,
            // a doubled `''` inside a single-quoted string is an
            // escaped literal `'`, not the end of the string. We
            // re-open the loop and add a literal `'`.
            LX2_QUOTE => {
                // c:1288 — `cmdpush(CS_QUOTE);`. Pops at c:1322 (inside
                // RCQUOTES `''` re-entry) and c:1330 (final break).
                cmdpush(CS_QUOTE as u8);
                // Single quoted string - everything literal until '
                add(Snull);
                loop {
                    let inner_loop_done = loop {
                        let ch = hgetc();
                        match ch {
                            Some('\'') => break false,
                            Some('\n') if !sub && isset(CSHJUNKIEQUOTES) => {
                                // CSHJUNKIEQUOTES: bare \n terminates.
                                // If preceded by `\`, the backslash is
                                // stripped (we approximate by peeking
                                // back at the buffer).
                                let last_was_bslash =
                                    LEX_LEXBUF.with_borrow(|b| b.as_str().ends_with('\\'));
                                if last_was_bslash {
                                    LEX_LEXBUF.with_borrow_mut(|b| {
                                        if let Some(s) = b.ptr.as_mut() {
                                            s.pop();
                                            b.len = s.len() as i32;
                                        }
                                    });
                                } else {
                                    break true; // terminate outer
                                }
                            }
                            Some(ch) => add(ch),
                            None => {
                                LEX_LEXSTOP.set(true);
                                unmatched = '\'';
                                peek = LEXERR;
                                break true;
                            }
                        }
                    };
                    if inner_loop_done || unmatched != '\0' {
                        break;
                    }
                    // c:1328 — RCQUOTES `''` → literal `'`.
                    if unset(RCQUOTES) {
                        break;
                    }
                    let e = hgetc();
                    if e != Some('\'') {
                        if let Some(e) = e {
                            hungetc(e);
                        }
                        LEX_LEXSTOP.set(false);
                        break;
                    }
                    add('\'');
                }
                // c:1330 — `cmdpop();` matches c:1288 push.
                cmdpop();
                if unmatched != '\0' {
                    break;
                }
                add(Snull);
            }

            // c:1334 — `case LX2_DQUOTE:` — `"`.
            //
            // c:1338-1340 — `cmdpush(CS_DQUOTE); c = dquote_parse('"',
            // sub); cmdpop();`
            LX2_DQUOTE => {
                // Double quoted string
                add(Dnull);
                cmdpush(CS_DQUOTE as u8);
                let r = dquote_parse('"', sub);
                cmdpop();
                if r.is_err() {
                    unmatched = '"';
                    if LEX_LEXFLAGS.get() & LEXFLAGS_ACTIVE == 0 {
                        peek = LEXERR;
                    }
                    break;
                }
                add(Dnull);
            }

            // c:1351 — `case LX2_BQUOTE:` — `` ` ``. Push/pop at
            // c:1352, 1379.
            //
            // c:1362 — `else if (!sub && isset(CSHJUNKIEQUOTES))
            // add(c);` — under CSHJUNKIEQUOTES, a `\<newline>` inside
            // backticks keeps the literal newline (line continuation
            // is NOT applied).
            //
            // c:1365 — `if (!sub && isset(CSHJUNKIEQUOTES) &&
            // c == '\n') break;` — under CSHJUNKIEQUOTES, a bare `\n`
            // inside backticks terminates the substitution.
            LX2_BQUOTE => {
                // Backtick command substitution
                add(Tick);
                cmdpush(CS_BQUOTE as u8);
                loop {
                    let ch = hgetc();
                    match ch {
                        Some('`') => break,
                        Some('\\') => {
                            let next = hgetc();
                            match next {
                                Some('\n') => {
                                    if !sub && isset(CSHJUNKIEQUOTES) {
                                        add('\n');
                                    }
                                    // Line continuation (default)
                                    continue;
                                }
                                Some(c) if c == '`' || c == '\\' || c == '$' => {
                                    add(Bnull);
                                    add(c);
                                }
                                Some(c) => {
                                    add('\\');
                                    add(c);
                                }
                                None => break,
                            }
                        }
                        Some('\n') if !sub && isset(CSHJUNKIEQUOTES) => {
                            // CSHJUNKIEQUOTES: bare \n terminates.
                            break;
                        }
                        Some(ch) => add(ch),
                        None => {
                            LEX_LEXSTOP.set(true);
                            unmatched = '`';
                            peek = LEXERR;
                            break;
                        }
                    }
                }
                // c:1379 — `cmdpop();` matches c:1352 push.
                cmdpop();
                if unmatched != '\0' {
                    break;
                }
                add(Tick);
            }

            // c:1044 — `case LX2_TILDE:` — `~`.
            LX2_TILDE => {
                add(Tilde);
            }

            // c:1162 — `case LX2_COMMA:` — `,`.
            //
            // c:1163 — `if (unset(IGNOREBRACES) && !sub && bct > in_brace_param)
            // c = Comma;` — only emit Comma (brace-expansion sep)
            // when IGNOREBRACES is off, not in substitution context,
            // and we are deeper than the current `${...}` brace.
            // Otherwise emit literal `,` (falls through to LX2_OTHER).
            LX2_COMMA if unset(IGNOREBRACES) && !sub && bct > in_brace_param => {
                add(Comma);
            }
            LX2_COMMA => {
                add(c);
            }

            // c:1394 — `case LX2_DASH:` — `-`.
            LX2_DASH => {
                add(Dash);
            }

            // c:1400 — `case LX2_BANG:` — `!`.
            LX2_BANG if brct > 0 => {
                add(Bang);
            }

            // c:967 — `case LX2_BREAK:` — `;` and `&`.
            //
            // Top-level terminator only when not inside `${...}`,
            // `(...)`, or `[...]`. Inside those, `;` is a delimiter
            // (e.g. field separator in `(@s.;.)`), not a statement
            // terminator. C zsh handles this via the same
            // bct/pct/brct accounting; we mirror it.
            LX2_BREAK if in_brace_param == 0 && pct == 0 && brct == 0 => {
                break;
            }
            LX2_BREAK => {
                add(c);
            }

            // c:1411 — `case LX2_OTHER:` — fallthrough.
            // C translates via `c = lextok2[STOUC(c)]` then adds —
            // turning `*` → `Star`, `?` → `Quest`, `#` → `Pound`,
            // `^` → `Hat` (other byte-token chars `{`, `[`, `$`,
            // `~` already have their own LX2_* arms).
            //
            // zshrs preserves the existing `\n` early-termination
            // behavior at top level — C handles `\n` in `gettok`,
            // not `gettokstr`, but our gettok hands off mid-word
            // through `lex_initial_other`, so `\n` can land here.
            LX2_OTHER => {
                if c == '\n' && in_brace_param == 0 && pct == 0 && brct == 0 {
                    break;
                }
                add(lextok2_get(c) as char);
            }

            _ => {
                add(c);
            }
        }

        c = match hgetc() {
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
        hungetc(c);
    }

    // c:1445-1446 — `if (unmatched && !(lexflags & LEXFLAGS_ACTIVE))
    //                  zerr("unmatched %c", unmatched);`
    if unmatched != '\0' && LEX_LEXFLAGS.get() & LEXFLAGS_ACTIVE == 0 {
        crate::ported::utils::zerr(&format!("unmatched {}", unmatched));
    }

    // c:1447-1453 — `zerr("closing brace expected");` when in_brace_param
    // is still open at end of token.
    if in_brace_param > 0 {
        crate::ported::utils::zerr("closing brace expected");
    }

    set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));
    peek
}

/// Check if a string is a valid assignment target (identifier or array ref).
///
/// zsh accepts identifier (`[A-Za-z_][A-Za-z0-9_]*`) optionally followed by
/// a `[...]` subscript. Bare digits are NOT a valid lvalue (rejected at
/// `if c.is_ascii_digit()` below — array index expressions like `arr[2]`
/// are caught by the subscript handler, not here). And the first char
/// must NOT be a zsh internal token byte — `$=foo` (where `$` becomes
/// the Stringg token 0x85) is parameter substitution with the `=` flag,
/// NOT an envstring assignment.
fn is_valid_assignment_target(s: &str) -> bool {
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
        if c == Inbrack || c == '[' {
            break;
        }
        if c == '+' {
            // foo+=value
            chars.next();
            return chars.peek().is_none() || chars.peek() == Some(&'=');
        }
        if !crate::ztype_h::iident(c as u8) && c != Stringg && !itok(c as u8) {
            return false;
        }
        has_ident = true;
        chars.next();
    }

    has_ident
}

/// Parse the body of a double-quoted string (or any context that
/// uses double-quote tokenization — `(( ))`, `${...}`, `$( ( ) )`).
/// Direct port of `dquote_parse` from `Src/lex.c:1486`. Reads chars
/// until `endchar` is seen at depth 0, handling escapes, `${...}`,
/// `$(...)`, backtick, `$((...))`, and inner `"..."`.
fn dquote_parse(endchar: char, sub: bool) -> Result<(), ()> {
    let mut pct = 0; // parenthesis count
    let mut brct = 0; // bracket count
    let mut bct = 0; // brace count (for ${...})
    let mut intick = false; // inside backtick
    let is_math = endchar == ')' || endchar == ']' || LEX_INFOR.get() > 0;

    // c:1643-1657 — on exit, drain matched-but-unpopped pushes:
    // every CS_BQUOTE (if `intick`), CS_BRACEPAR + (CS_CURSH when
    // it was set) (for each remaining bct level). Wrapped via
    // closure so success + every early Err goes through cleanup.
    let cleanup = |intick: bool, bct: i32| {
        if intick {
            cmdpop();
        }
        for _ in 0..bct {
            cmdpop(); // CS_BRACEPAR for each remaining `{`.
        }
    };

    loop {
        let c = hgetc();
        let c = match c {
            Some(c) if c == endchar && !intick && bct == 0 => {
                if is_math && (pct > 0 || brct > 0) {
                    add(c);
                    if c == ')' {
                        pct -= 1;
                    } else if c == ']' {
                        brct -= 1;
                    }
                    continue;
                }
                cleanup(intick, bct);
                return Ok(());
            }
            Some(c) => c,
            None => {
                LEX_LEXSTOP.set(true);
                cleanup(intick, bct);
                return Err(());
            }
        };

        match c {
            // c:1499 — `case '\\':`.
            '\\' => {
                let next = hgetc();
                match next {
                    Some('\n') => {
                        // c:1515 — `else if (sub || unset(CSHJUNKIEQUOTES)
                        // || endchar != '"') continue;` — under
                        // CSHJUNKIEQUOTES inside `"..."`, `\<newline>`
                        // is NOT line continuation; it falls through
                        // to add literal `\n`.
                        if sub || unset(CSHJUNKIEQUOTES) || endchar != '"' {
                            continue;
                        }
                        add('\n');
                    }
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
                        add(Bnull);
                        add(c);
                    }
                    Some(c) => {
                        add('\\');
                        hungetc(c);
                        continue;
                    }
                    None => {
                        add('\\');
                    }
                }
            }

            // c:1517 — `case '\n': err = !sub && isset(CSHJUNKIEQUOTES)
            // && endchar == '"';` — under CSHJUNKIEQUOTES, a bare `\n`
            // inside `"..."` is an error (unterminated string).
            '\n' if !sub && isset(CSHJUNKIEQUOTES) && endchar == '"' => {
                return Err(());
            }

            '$' => {
                if intick {
                    add(c);
                    continue;
                }
                let next = hgetc();
                match next {
                    Some('(') => {
                        add(Qstring);
                        match cmd_or_math_sub() {
                            CMD_OR_MATH_CMD => add(Outpar),
                            CMD_OR_MATH_MATH => add(Outparmath),
                            CMD_OR_MATH_ERR | _ => return Err(()),
                        }
                    }
                    Some('[') => {
                        // c:1541 — `cmdpush(CS_MATHSUBST); err =
                        // dquote_parse(']', sub); cmdpop();`
                        add(Stringg);
                        add(Inbrack);
                        cmdpush(CS_MATHSUBST as u8);
                        let r = dquote_parse(']', sub);
                        cmdpop();
                        r?;
                        add(Outbrack);
                    }
                    Some('{') => {
                        // c:1548 — `cmdpush(CS_BRACEPAR); bct++;`
                        add(Qstring);
                        add(Inbrace);
                        cmdpush(CS_BRACEPAR as u8);
                        bct += 1;
                    }
                    Some('$') => {
                        add(Qstring);
                        add('$');
                    }
                    _ => {
                        if let Some(next) = next {
                            hungetc(next);
                        }
                        LEX_LEXSTOP.set(false);
                        add(Qstring);
                    }
                }
            }

            '}' => {
                if intick || bct == 0 {
                    add(c);
                } else {
                    // c:1575/1577 — `cmdpop()` for inner brace, plus
                    // matching CS_BRACEPAR pop on the outermost
                    // closer.
                    add(Outbrace);
                    cmdpop();
                    bct -= 1;
                }
            }

            // c:1583 — `case '`':` — backtick toggle.
            // c:1585 — `cmdpush(CS_BQUOTE)` on entry, c:1588
            // `cmdpop()` on exit.
            '`' => {
                add(Qtick);
                if intick {
                    cmdpop();
                } else {
                    cmdpush(CS_BQUOTE as u8);
                }
                intick = !intick;
            }

            '(' => {
                if !is_math || bct == 0 {
                    pct += 1;
                }
                add(c);
            }

            ')' => {
                if !is_math || bct == 0 {
                    if pct == 0 && is_math {
                        return Err(());
                    }
                    pct -= 1;
                }
                add(c);
            }

            '[' => {
                if !is_math || bct == 0 {
                    brct += 1;
                }
                add(c);
            }

            ']' => {
                if !is_math || bct == 0 {
                    if brct == 0 && is_math {
                        return Err(());
                    }
                    brct -= 1;
                }
                add(c);
            }

            '"' => {
                if intick || (endchar != '"' && bct == 0) {
                    add(c);
                } else if bct > 0 {
                    // c:1620 — `cmdpush(CS_DQUOTE); err =
                    // dquote_parse('"', sub); cmdpop();`
                    add(Dnull);
                    cmdpush(CS_DQUOTE as u8);
                    let r = dquote_parse('"', sub);
                    cmdpop();
                    r?;
                    add(Dnull);
                } else {
                    return Err(());
                }
            }

            _ => {
                add(c);
            }
        }
    }
}

/// Determine if (( is arithmetic or command
/// Decide whether `( ... )` after a `$` is a math expression
/// `$((...))` or a command substitution `$(...)`. Direct port of
/// zsh/Src/lex.c:495 `cmd_or_math`. Tries dquote_parse first;
/// if it succeeds AND the next char is `)` (closing the second
/// paren of `(( ))`), it's math. Otherwise rewinds and treats as
/// a command substitution.
fn cmd_or_math() -> i32 {
    let oldlen = LEX_LEXBUF.with_borrow(|b| b.buf_len());

    // c:501 — `cmdpush(cs_type);` — the C source takes a `cs_type`
    // arg (CS_MATH or CS_MATHSUBST); zshrs's lone caller (gettok
    // LX1_INPAR `((`) uses CS_MATH. The matching `cmdpop()` fires
    // both on the math path (after success) and on the rewind path
    // (before falling through to skipcomm, which has its own
    // CS_CMDSUBST push/pop).
    cmdpush(CS_MATH as u8);

    // Per lex.c:498-518 — `cmd_or_math` calls `dquote_parse(')')`
    // which fills lexbuf with ONLY the inner expression, then checks
    // for the closing `)`. The surrounding `((` / `))` are NOT added
    // to lexbuf. zshrs previously added Inpar + '(' before dquote and
    // ')' after, polluting DINPAR's tokstr with the literal parens.
    // Removed to match C exactly.
    if dquote_parse(')', false).is_err() {
        // c:506 — `cmdpop();` before rewind to command-parse path.
        cmdpop();
        // Back up and try as command
        while LEX_LEXBUF.with_borrow(|b| b.buf_len()) > oldlen {
            if let Some(c) = LEX_LEXBUF.with_borrow_mut(|b| b.pop()) {
                hungetc(c);
            }
        }
        hungetc('(');
        LEX_LEXSTOP.set(false);
        return if skipcomm().is_err() {
            CMD_OR_MATH_ERR
        } else {
            CMD_OR_MATH_CMD
        };
    }

    // Check for closing ) — matches C lex.c:511-512: success-with-`)`
    // means `((..))` was math. Don't add `)` to lexbuf.
    let c = hgetc();
    if c == Some(')') {
        // c:506 — `cmdpop();` on math success.
        cmdpop();
        return CMD_OR_MATH_MATH;
    }

    // c:506 — `cmdpop();` before rewind to command-parse path.
    cmdpop();
    // Not math, back up
    if let Some(c) = c {
        hungetc(c);
    }
    LEX_LEXSTOP.set(false);

    // Back up token
    while LEX_LEXBUF.with_borrow(|b| b.buf_len()) > oldlen {
        if let Some(c) = LEX_LEXBUF.with_borrow_mut(|b| b.pop()) {
            hungetc(c);
        }
    }
    hungetc('(');

    if skipcomm().is_err() {
        CMD_OR_MATH_ERR
    } else {
        CMD_OR_MATH_CMD
    }
}

/// Parse `$(...)` or `$((...))` after the `$` has been consumed.
/// Direct port of zsh/Src/lex.c:540 `cmd_or_math_sub`. Reads
/// the next char to discriminate: a leading `(` plus successful
/// math parse via `cmd_or_math` → arithmetic substitution (with
/// the open-paren retroactively rewritten to Inparmath); else
/// command substitution via skipcomm.
fn cmd_or_math_sub() -> i32 {
    loop {
        let c = hgetc();
        if c == Some('\\') {
            let c2 = hgetc();
            if c2 != Some('\n') {
                if let Some(c2) = c2 {
                    hungetc(c2);
                }
                hungetc('\\');
                LEX_LEXSTOP.set(false);
                return if skipcomm().is_err() {
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
            add(Inpar);
            add('(');

            if dquote_parse(')', false).is_ok() {
                let c2 = hgetc();
                if c2 == Some(')') {
                    add(')');
                    return CMD_OR_MATH_MATH;
                }
                if let Some(c2) = c2 {
                    hungetc(c2);
                }
            }

            // Not math, restore and parse as command
            while LEX_LEXBUF.with_borrow(|b| b.buf_len()) > lexpos {
                if let Some(ch) = LEX_LEXBUF.with_borrow_mut(|b| b.pop()) {
                    hungetc(ch);
                }
            }
            hungetc('(');
            LEX_LEXSTOP.set(false);
        } else {
            if let Some(c) = c {
                hungetc(c);
            }
            LEX_LEXSTOP.set(false);
        }

        return if skipcomm().is_err() {
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
fn skipcomm() -> Result<(), ()> {
    use crate::ported::zsh_h::{ZCONTEXT_LEX, ZCONTEXT_PARSE};
    // c:2094-2225 — `skipcomm`. Captures the verbatim text of a
    // `$(...)` / `<(...)` / `>(...)` body into the parent token via
    // C's lex_add_raw / lexbuf_raw mechanism (lex.c:2098-2149):
    //   1. add(Inpar) — outer lexbuf gets `(` (the marker form).
    //   2. Copy outer tokstr/lexbuf into new_tokstr/new_lexbuf so the
    //      raw buffer starts seeded with the prefix already lexed
    //      (e.g. `$(` plus anything before).
    //   3. zcontext_save_partial — saves AND resets lexbuf, lexbuf_raw,
    //      lex_add_raw to fresh.
    //   4. tokstr_raw = new_tokstr; lexbuf_raw = new_lexbuf — the raw
    //      buffer now mirrors the outer's pre-call lexbuf.
    //   5. lex_add_raw = old + 1 — turns on raw-recording so every
    //      char hgetc reads also lands in lexbuf_raw.
    //   6. Walk the inner body via hgetc/add. lexbuf gets the
    //      throw-away tokenized form; lexbuf_raw accumulates the
    //      verbatim chars.
    //   7. Capture new_tokstr/new_lexbuf from the raw buffer.
    //   8. zcontext_restore_partial restores outer lex state.
    //   9. If outer lex_add_raw == 0: tokstr = new_tokstr; lexbuf =
    //      new_lexbuf — outer's lexbuf is REPLACED with the captured
    //      raw body (which already contains the `$(` prefix from step
    //      4 plus the body chars). If outer lex_add_raw != 0 (nested
    //      cmd-sub), propagate the raw vars.
    let new_lex_add_raw = LEX_LEX_ADD_RAW.get() + 1;
    let outer_was_recording = LEX_LEX_ADD_RAW.get() != 0;

    cmdpush(CS_CMDSUBST as u8);
    add(Inpar);

    // c:2096-2143 — save outer tokstr/lexbuf into the variables that
    // will become tokstr_raw/lexbuf_raw post-save.
    let new_tokstr_init: Option<String>;
    let new_lexbuf_init_ptr: Option<String>;
    let new_lexbuf_init_siz: i32;
    let new_lexbuf_init_len: i32;
    if outer_was_recording {
        // Nested: propagate the existing raw buffers.
        new_tokstr_init = LEX_TOKSTR_RAW.with_borrow_mut(|t| t.take());
        let (p, s, l) = LEX_LEXBUF_RAW.with_borrow_mut(|b| {
            (b.ptr.take(), b.siz, b.len)
        });
        new_lexbuf_init_ptr = p;
        new_lexbuf_init_siz = s;
        new_lexbuf_init_len = l;
    } else {
        // Top-level: seed raw with current tokstr/lexbuf.
        new_tokstr_init = tokstr();
        let (p, s, l) = LEX_LEXBUF.with_borrow(|b| {
            (b.ptr.clone(), b.siz, b.len)
        });
        new_lexbuf_init_ptr = p;
        new_lexbuf_init_siz = s;
        new_lexbuf_init_len = l;
    }

    crate::ported::context::zcontext_save_partial(ZCONTEXT_LEX | ZCONTEXT_PARSE);
    crate::ported::hist::hist_in_word(1);

    // c:2147-2149 — install seeded raw buffers + enable recording.
    set_tokstr(new_tokstr_init);
    LEX_LEXBUF.with_borrow_mut(|b| {
        b.ptr = new_lexbuf_init_ptr.clone();
        b.siz = if new_lexbuf_init_siz == 0 { 256 } else { new_lexbuf_init_siz };
        b.len = new_lexbuf_init_len;
    });
    LEX_TOKSTR_RAW.with_borrow_mut(|t| *t = tokstr());
    LEX_LEXBUF_RAW.with_borrow_mut(|b| {
        b.ptr = new_lexbuf_init_ptr;
        b.siz = if new_lexbuf_init_siz == 0 { 256 } else { new_lexbuf_init_siz };
        b.len = new_lexbuf_init_len;
    });
    LEX_LEX_ADD_RAW.set(new_lex_add_raw);

    // RAII: cleanup on every exit path. Captures the raw body, restores
    // outer lex state, then (if outer wasn't recording) overwrites the
    // restored outer lexbuf with the raw body — the trick that makes
    // the parent token contain the verbatim `$(...)` text.
    struct SkipcommGuard {
        outer_was_recording: bool,
    }
    impl Drop for SkipcommGuard {
        fn drop(&mut self) {
            // c:2185-2186 — capture the raw form before restore.
            let new_tokstr = LEX_TOKSTR_RAW.with_borrow_mut(|t| t.take());
            let (new_lexbuf_ptr, new_lexbuf_siz, new_lexbuf_len) =
                LEX_LEXBUF_RAW.with_borrow_mut(|b| (b.ptr.take(), b.siz, b.len));
            let new_lexstop = LEX_LEXSTOP.get();

            crate::ported::hist::hist_in_word(0);
            crate::ported::context::zcontext_restore_partial(ZCONTEXT_LEX | ZCONTEXT_PARSE);

            // c:2196-2217 — splice raw back into outer lexbuf, or
            // propagate to outer raw if outer was recording.
            if self.outer_was_recording {
                LEX_TOKSTR_RAW.with_borrow_mut(|t| *t = new_tokstr);
                LEX_LEXBUF_RAW.with_borrow_mut(|b| {
                    b.ptr = new_lexbuf_ptr;
                    b.siz = new_lexbuf_siz;
                    b.len = new_lexbuf_len;
                });
            } else {
                // c:2204-2207 — strip the trailing `)` that hgetc
                // recorded into the raw buffer (closing paren).
                let mut final_ptr = new_lexbuf_ptr;
                let mut final_len = new_lexbuf_len;
                if !new_lexstop {
                    if let Some(ref mut s) = final_ptr {
                        if s.ends_with(')') {
                            s.pop();
                            final_len -= 1;
                        }
                    }
                }
                set_tokstr(final_ptr.clone());
                LEX_LEXBUF.with_borrow_mut(|b| {
                    b.ptr = final_ptr;
                    b.siz = new_lexbuf_siz;
                    b.len = final_len;
                });
            }
            cmdpop();
        }
    }
    let _guard = SkipcommGuard { outer_was_recording };

    let mut pct = 1;
    let mut start = true;

    loop {
        let c = hgetc();
        let c = match c {
            Some(c) => c,
            None => {
                LEX_LEXSTOP.set(true);
                return Err(());
            }
        };

        let iswhite = crate::ztype_h::inblank(c as u8);

        match c {
            '(' => {
                pct += 1;
                add(c);
            }
            ')' => {
                pct -= 1;
                if pct == 0 {
                    return Ok(());
                }
                add(c);
            }
            '\\' => {
                add(c);
                if let Some(c) = hgetc() {
                    add(c);
                }
            }
            '\'' => {
                add(c);
                loop {
                    let ch = hgetc();
                    match ch {
                        Some('\'') => {
                            add('\'');
                            break;
                        }
                        Some(ch) => add(ch),
                        None => {
                            LEX_LEXSTOP.set(true);
                            return Err(());
                        }
                    }
                }
            }
            '"' => {
                add(c);
                loop {
                    let ch = hgetc();
                    match ch {
                        Some('"') => {
                            add('"');
                            break;
                        }
                        Some('\\') => {
                            add('\\');
                            if let Some(ch) = hgetc() {
                                add(ch);
                            }
                        }
                        Some(ch) => add(ch),
                        None => {
                            LEX_LEXSTOP.set(true);
                            return Err(());
                        }
                    }
                }
            }
            '`' => {
                add(c);
                loop {
                    let ch = hgetc();
                    match ch {
                        Some('`') => {
                            add('`');
                            break;
                        }
                        Some('\\') => {
                            add('\\');
                            if let Some(ch) = hgetc() {
                                add(ch);
                            }
                        }
                        Some(ch) => add(ch),
                        None => {
                            LEX_LEXSTOP.set(true);
                            return Err(());
                        }
                    }
                }
            }
            '#' if start => {
                add(c);
                // Skip comment to end of line
                loop {
                    let ch = hgetc();
                    match ch {
                        Some('\n') => {
                            add('\n');
                            break;
                        }
                        Some(ch) => add(ch),
                        None => break,
                    }
                }
            }
            _ => {
                add(c);
            }
        }

        start = iswhite;
    }
}

/// Lex next token AND update per-context flags. Direct port of
/// zsh/Src/lex.c:317 `ctxtlex`. The post-token state machine
/// at lex.c:322-358 sets `incmdpos` based on the token shape:
/// list separators / pipes / control keywords reset to cmd-pos;
/// word-shaped tokens leave cmd-pos. Redirections (lex.c:361-368)
/// stash prior incmdpos and force the redir target to non-cmd-pos.
pub fn ctxtlex() {
    // lex.c:319 — static `oldpos` cache for redir-target restore
    // is captured per-call here as `oldpos` below (zshrs's parser
    // re-enters ctxtlex per token, no need for static persistence).

    // lex.c:321 — `zshlex();` to advance to the next token.
    zshlex();

    // c:322-358 — post-token incmdpos switch. C lists the
    // arms exactly as enumerated below; `intypeset` is NOT set
    // here in C (it lives in parse.c:1932/2042/2047, ported in
    // parse.rs at the typeset-call sites).
    match tok() {
        // c:323-343 — separators / openers / conjunctions /
        // control keywords — back into cmd-pos so the next token
        // can be a fresh command.
        SEPER | NEWLIN | SEMI | DSEMI | SEMIAMP | SEMIBAR | AMPER | AMPERBANG | INPAR_TOK
        | INBRACE_TOK | DBAR | DAMPER | BAR_TOK | BARAMP | INOUTPAR | DOLOOP | THEN | ELIF
        | ELSE | DOUTBRACK => {
            LEX_INCMDPOS.set(true);
        }
        // c:345-353 — word/value-shaped tokens leave cmd-pos.
        STRING_LEX | TYPESET | ENVARRAY | OUTPAR_TOK | CASE | DINBRACK => {
            LEX_INCMDPOS.set(false);
        }
        // c:354-357 — `default: break;` — keep compiler happy.
        _ => {}
    }

    // lex.c:359-360 — `infor` decay. FOR sets infor=2 so the next
    // DINPAR can detect c-style for. After any non-DINPAR, decay
    // to 0 (or back to 2 if we just saw FOR again).
    if tok() != DINPAR {
        LEX_INFOR.set(if tok() == FOR { 2 } else { 0 });
    }

    // lex.c:361-368 — redir-target context dance. After consuming
    // a redir operator, the following token (the file path) sees
    // incmdpos=0 even when its inherent shape would put it back
    // in cmd-pos. After the redir target, restore from oldpos
    // (struct field — must persist across zshlex calls).
    if IS_REDIROP(tok()) || tok() == FOR || tok() == FOREACH || tok() == SELECT {
        LEX_INREDIR.set(true);
        LEX_OLDPOS.set(LEX_INCMDPOS.get());
        LEX_INCMDPOS.set(false);
    } else if LEX_INREDIR.get() {
        LEX_INCMDPOS.set(LEX_OLDPOS.get());
        LEX_INREDIR.set(false);
    }
}

/// Mark the current word as the one ZLE was looking for. Direct
/// port of `gotword(void)` from `Src/lex.c:1882`. Computes the
/// new-word-end (`nwe`) and new-word-begin (`nwb`) line positions
/// based on `zlemetall`, `inbufct`, `addedx`, and `wordbeg`, then
/// — if the cursor (`zlemetacs`) falls inside that range — writes
/// `wb`/`we` and clears `lexflags`.
pub fn gotword() {
    use std::sync::atomic::Ordering;
    let zlemetacs = crate::ported::zle::compcore::ZLEMETACS.load(Ordering::SeqCst);
    let zlemetall = crate::ported::zle::compcore::ZLEMETALL.load(Ordering::SeqCst);
    let addedx = crate::ported::zle::compcore::ADDEDX.load(Ordering::SeqCst);
    let inbufct = crate::ported::input::inbufct.with(|c| c.get());
    let wordbeg = LEX_WORDBEG.get();

    // c:1884 — `int nwe = zlemetall + 1 - inbufct + (addedx == 2 ? 1 : 0);`
    let nwe = zlemetall + 1 - inbufct + if addedx == 2 { 1 } else { 0 };
    // c:1885 — `if (zlemetacs <= nwe)`
    if zlemetacs <= nwe {
        // c:1886 — `int nwb = zlemetall - wordbeg + addedx;`
        let nwb = zlemetall - wordbeg + addedx;
        // c:1887-1893 — `if (zlemetacs >= nwb) { wb = nwb; we = nwe; }
        // else { wb = zlemetacs + addedx; if (we < wb) we = wb; }`.
        if zlemetacs >= nwb {
            crate::ported::zle::compcore::WB.store(nwb, Ordering::SeqCst);
            crate::ported::zle::compcore::WE.store(nwe, Ordering::SeqCst);
        } else {
            let wb_new = zlemetacs + addedx;
            crate::ported::zle::compcore::WB.store(wb_new, Ordering::SeqCst);
            let we_cur = crate::ported::zle::compcore::WE.load(Ordering::SeqCst);
            if we_cur < wb_new {
                crate::ported::zle::compcore::WE.store(wb_new, Ordering::SeqCst);
            }
        }
        // c:1895 — `lexflags = 0;`
        LEX_LEXFLAGS.set(0);
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
/// Direct port of zsh/Src/lex.c:581 `isnumglob`. C source uses
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

/// Port of `parsestrnoerr(char **s)` from `Src/lex.c:1713`.
///
/// C body:
/// ```c
/// zcontext_save();
/// untokenize(*s);
/// inpush(dupstring_wlen(*s, l), 0, NULL);
/// strinbeg(0);
/// lexbuf.len = 0; lexbuf.ptr = tokstr = *s; lexbuf.siz = l + 1;
/// err = dquote_parse('\0', 1);
/// if (tokstr) *s = tokstr;
/// *lexbuf.ptr = '\0';
/// strinend();
/// inpop();
/// zcontext_restore();
/// return err;
/// ```
///
/// Drives the real `dquote_parse` (with `endchar='\0'`, `sub=true`)
/// through the nested-lex-context machinery so `${...}`, `$(...)`,
/// `$((...))`, backticks, etc. tokenize recursively the same way
/// they do during a normal command parse. Returns the tokenized
/// string on success.
pub fn parsestrnoerr(s: &str) -> Result<String, String> {
    let untok = untokenize(s);                                                // c:1716 `untokenize(*s);`
    let dup = crate::ported::string::dupstring_wlen(&untok, untok.len());     // c:1717
    // c:1715 `zcontext_save();`
    crate::ported::context::zcontext_save();
    // c:1717 `inpush(dupstring_wlen(*s, l), 0, NULL);`
    crate::ported::input::inpush(&dup, 0, None);
    // c:1718 `strinbeg(0);`
    crate::ported::hist::strinbeg(0);
    // c:1719-1721 — seed lexbuf with the input string so dquote_parse's
    // `add()` writes append onto our copy. `lexbuf.ptr/siz/len` are
    // reset; tokstr is aliased to the buffer.
    LEX_LEXBUF.with_borrow_mut(|b| {
        b.ptr = Some(String::with_capacity(untok.len() + 1));
        b.siz = (untok.len() + 1) as i32;
        b.len = 0;
    });
    set_tokstr(None);
    // c:1722 `err = dquote_parse('\0', 1);`
    let parse_err = dquote_parse('\0', true).is_err();
    // c:1723-1725 — `if (tokstr) *s = tokstr; *lexbuf.ptr = '\0';`
    let result = LEX_LEXBUF.with_borrow(|b| b.as_str().to_string());
    // c:1726 `strinend();`
    crate::ported::hist::strinend();
    // c:1727 `inpop();`
    crate::ported::input::inpop();
    // c:1729 `zcontext_restore();`
    crate::ported::context::zcontext_restore();
    if parse_err {
        // C parsestrnoerr (lex.c:1713) returns the offending char so the
        // caller (parsestr at lex.c:1694) can format `zerr("parse error
        // near \`%c'", err)`. zshrs returns Err(message); the diagnostic
        // already went through zerr at the dquote_parse / gettokstr
        // failure site, so a generic message is sufficient here.
        Err("parse error".to_string())
    } else {
        Ok(result)
    }
}

/// Tokenize a string as if in double quotes (error-reporting variant).
/// Direct port of `parsestr(char **s)` from `Src/lex.c:1694`. C
/// source: `if ((err = parsestrnoerr(s))) { untokenize(*s); ...
/// zerr("parse error near `%c'", err); tok = LEXERR; }`. zshrs's
/// wrapper preserves the Result and lets the caller emit the
/// diagnostic.
pub fn parsestr(s: &str) -> Result<String, String> {
    parsestrnoerr(s)
}

/// Parse a subscript in string s. Return the position after the
/// closing bracket, or None on error.
///
/// Direct port of zsh/Src/lex.c:1743 `parse_subscript`. The C
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
    // c:1746 `if (!*s || *s == endchar) return 0;`
    if s.is_empty() || s.starts_with(endchar) {
        return None;
    }
    let l = s.len();
    let untok = untokenize(s);                                                // c:1749 `untokenize(t = dupstring_wlen(s, l));`
    let dup = crate::ported::string::dupstring_wlen(&untok, untok.len());
    // c:1748 `zcontext_save();`
    crate::ported::context::zcontext_save();
    // c:1750 `inpush(t, 0, NULL);`
    crate::ported::input::inpush(&dup, 0, None);
    // c:1751 `strinbeg(0);`
    crate::ported::hist::strinbeg(0);
    // c:1763-1765 — seed lexbuf and run dquote_parse with the
    // caller's `endchar` + `sub=false` (zshrs's API omits the C `sub`
    // arg — all current callers pass 0).
    LEX_LEXBUF.with_borrow_mut(|b| {
        b.ptr = Some(String::with_capacity(l + 1));
        b.siz = (l + 1) as i32;
        b.len = 0;
    });
    let parse_err = dquote_parse(endchar, false).is_err();
    let toklen = LEX_LEXBUF.with_borrow(|b| b.len) as usize;
    // c:1779 `strinend();` / c:1780 `inpop();` / c:1782
    // `zcontext_restore();`
    crate::ported::hist::strinend();
    crate::ported::input::inpop();
    crate::ported::context::zcontext_restore();
    if parse_err {
        return None;
    }
    Some(toklen)
}

/// Tokenize a string as if it were a normal command-line argument
/// but it may contain separators. Used for ${...%...} substitutions.
///
/// Direct port of zsh/Src/lex.c:1796 `parse_subst_string`.
/// zsh's version sets `noaliases = 1` + `lexflags = 0` + uses
/// zcontext_save/inpush/strinbeg → dquote_parse('\0', 1) →
/// strinend/inpop/zcontext_restore. zshrs's standalone walker
/// produces the same Bnull/Snull/Dnull/Inpar/Inbrack markers
/// without re-entering the lexer.
///
/// zshrs port note: the C source returns int (0=ok, char value =
/// where it stopped on error); zshrs returns Result<String,String>
/// returning the tokenized text directly. Lossy for callers that
/// need to know the exact stop position, but nothing in zshrs's
/// expansion layer uses that yet.
pub fn parse_subst_string(s: &str) -> Result<String, String> {
    // c:1802 `if (!*s || !strcmp(s, nulstring)) return 0;`
    if s.is_empty() {
        return Ok(String::new());
    }
    let l = s.len();
    let untok = untokenize(s);                                                // c:1804
    let dup = crate::ported::string::dupstring_wlen(&untok, untok.len());
    // c:1803 `zcontext_save();`
    crate::ported::context::zcontext_save();
    // c:1805 `inpush(dupstring_wlen(s, l), 0, NULL);`
    crate::ported::input::inpush(&dup, 0, None);
    // c:1806 `strinbeg(0);`
    crate::ported::hist::strinbeg(0);
    // c:1807-1809 — seed lexbuf with the input string.
    LEX_LEXBUF.with_borrow_mut(|b| {
        b.ptr = Some(String::with_capacity(l + 1));
        b.siz = (l + 1) as i32;
        b.len = 0;
    });
    set_tokstr(None);
    // c:1810 `c = hgetc();` / c:1811 `ctok = gettokstr(c, 1);`
    let c0 = hgetc();
    let ctok = match c0 {
        Some(ch) => gettokstr(ch, true),
        None => LEXERR,
    };
    use std::sync::atomic::Ordering;
    let saw_err = crate::ported::utils::errflag.load(Ordering::Relaxed) != 0;
    let result = LEX_LEXBUF.with_borrow(|b| b.as_str().to_string());
    // c:1813 `strinend();`
    crate::ported::hist::strinend();
    // c:1814 `inpop();`
    crate::ported::input::inpop();
    // c:1816 `zcontext_restore();`
    crate::ported::context::zcontext_restore();
    if ctok == LEXERR || saw_err {
        // Diagnostic already emitted via zerr at the failure site.
        return Err("parse error".to_string());
    }
    Ok(result)
}

/// Untokenize a string - convert tokenized chars back to original
///
/// Port of untokenize(char *s) from exec.c (but used by lexer too)
/// Like `untokenize`, but maps Snull → `'` and Dnull → `"` instead of
/// stripping them. Used by callers that need the source form including
/// quoting (e.g. arithmetic-substitution detection in compile_zsh).
pub fn untokenize_preserve_quotes(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        let cu = c as u32;
        if (0x83..=0x9f).contains(&cu) {
            match c {
                c if c == Pound => result.push('#'),
                c if c == Stringg => result.push('$'),
                c if c == Hat => result.push('^'),
                c if c == Star => result.push('*'),
                c if c == Inpar => result.push('('),
                c if c == Outpar => result.push(')'),
                c if c == Inparmath => result.push('('),
                c if c == Outparmath => result.push(')'),
                c if c == Qstring => result.push('$'),
                c if c == Equals => result.push('='),
                c if c == Bar => result.push('|'),
                c if c == Inbrace => result.push('{'),
                c if c == Outbrace => result.push('}'),
                c if c == Inbrack => result.push('['),
                c if c == Outbrack => result.push(']'),
                c if c == Tick => result.push('`'),
                c if c == Inang => result.push('<'),
                c if c == Outang => result.push('>'),
                c if c == OutangProc => result.push('>'),
                c if c == Quest => result.push('?'),
                c if c == Tilde => result.push('~'),
                c if c == Qtick => result.push('`'),
                c if c == Comma => result.push(','),
                c if c == Dash => result.push('-'),
                c if c == Bang => result.push('!'),
                c if c == Snull => result.push('\''),
                c if c == Dnull => result.push('"'),
                c if c == Bnull => result.push('\\'),
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
/// Port of `getkeystring(char *s, int *len, int how, int *misc)` from Src/utils.c:6915 with the
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
        if c == Snull {
            return (out, i);
        }
        if c == Bnull {
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
        // Bnull). Anything in that range needs un-mapping before display
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
            if c == Qstring && i + 1 < chars.len() && chars[i + 1] == Snull {
                let (decoded, end) = getkeystring_dollar_quote(&chars, i + 2);
                result.push_str(&decoded);
                // `end` points at the closing `Snull` (or end of
                // string if unterminated); skip past it.
                i = if end < chars.len() { end + 1 } else { end };
                continue;
            }
            // Convert token back to original character
            match c {
                c if c == Pound => result.push('#'),
                c if c == Stringg => result.push('$'),
                c if c == Hat => result.push('^'),
                c if c == Star => result.push('*'),
                c if c == Inpar => result.push('('),
                c if c == Outpar => result.push(')'),
                c if c == Inparmath => result.push('('),
                c if c == Outparmath => result.push(')'),
                c if c == Qstring => result.push('$'),
                c if c == Equals => result.push('='),
                c if c == Bar => result.push('|'),
                c if c == Inbrace => result.push('{'),
                c if c == Outbrace => result.push('}'),
                c if c == Inbrack => result.push('['),
                c if c == Outbrack => result.push(']'),
                c if c == Tick => result.push('`'),
                c if c == Inang => result.push('<'),
                c if c == Outang => result.push('>'),
                c if c == OutangProc => result.push('>'),
                c if c == Quest => result.push('?'),
                c if c == Tilde => result.push('~'),
                c if c == Qtick => result.push('`'),
                c if c == Comma => result.push(','),
                c if c == Dash => result.push('-'),
                c if c == Bang => result.push('!'),
                c if c == Snull || c == Dnull || c == Bnull => {
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
/// Mirrors C `itok(c)` (zsh.h). zsh's token markers live in the
/// META range 0x83..=0x9f (Pound..Bnull at zsh.h:160-188). Earlier
/// implementation checked `< 32` (control chars) which is wrong for
/// zsh — none of its tokens land there. The bad check made
/// `exalias` skip the `untokenize(tokstr)` path for any token
/// containing markers (e.g. `[[` lexes as `Inbrack Inbrack` =
/// `\u{91}\u{91}`), so the reswdtab lookup compared raw marker
/// bytes against the literal `"[["` key and never promoted to
/// DINBRACK. Same hit `{`/`}`, `$`, `*`, `?`, etc.
pub fn has_token(s: &str) -> bool {
    s.chars().any(|c| {
        let cu = c as u32;
        (0x83..=0x9f).contains(&cu)
    })
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
        let _ = lex_init("echo hello");
        zshlex();
        assert_eq!(tok(), STRING_LEX);
        assert_eq!(tokstr(), Some("echo".to_string()));

        zshlex();
        assert_eq!(tok(), STRING_LEX);
        assert_eq!(tokstr(), Some("hello".to_string()));

        zshlex();
        assert_eq!(tok(), ENDINPUT);
    }

    #[test]
    fn test_pipeline() {
        let _ = lex_init("ls | grep foo");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), BAR_TOK);

        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
    }

    #[test]
    fn test_redirections() {
        let _ = lex_init("echo > file");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), OUTANG_TOK);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
    }

    #[test]
    fn test_heredoc() {
        let _ = lex_init("cat << EOF");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), DINANG);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
    }

    #[test]
    fn test_single_quotes() {
        let _ = lex_init("echo 'hello world'");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
        // Should contain Snull markers around literal content
        assert!(tokstr().is_some());
    }

    #[test]
    fn test_function_tokens() {
        // C zsh's par_funcdef (parse.c:1681, 1717) explicitly toggles
        // `incmdpos` around the function header: 0 before reading the
        // name, 1 before the body opener. The Rust zshlex auto-updates
        // cmdpos C-ctxtlex-style (lex.rs:1492-1553), so a STRING name
        // sets incmdpos=false. To assert the body opener lexes as
        // INBRACE (rather than STRING with the Inbrace marker), we
        // have to mimic par_funcdef's incmdpos=1 reset by hand.
        let _ = lex_init("function foo { }");
        zshlex();
        assert_eq!(tok(), FUNC, "expected Func, got {:?}", tok());

        zshlex();
        assert_eq!(
            tok(),
            STRING_LEX,
            "expected String for 'foo', got {:?}",
            tok()
        );
        assert_eq!(tokstr(), Some("foo".to_string()));

        // par_funcdef equivalent: parse.c:1717 `incmdpos = 1;` before
        // the body opener.
        set_incmdpos(true);
        zshlex();
        assert_eq!(
            tok(),
            INBRACE_TOK,
            "expected Inbrace, got {:?} tokstr={:?}",
            tok(),
            tokstr()
        );

        zshlex();
        assert_eq!(
            tok(),
            OUTBRACE_TOK,
            "expected Outbrace, got {:?} tokstr={:?} incmdpos={}",
            tok(),
            tokstr(),
            incmdpos()
        );
    }

    #[test]
    fn test_double_quotes() {
        let _ = lex_init("echo \"hello $name\"");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
        // Should contain tokenized content
        assert!(tokstr().is_some());
    }

    #[test]
    fn test_command_substitution() {
        let _ = lex_init("echo $(pwd)");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
    }

    #[test]
    fn test_env_assignment() {
        let _ = lex_init("FOO=bar echo");
        set_incmdpos(true);
        zshlex();
        assert_eq!(tok(), ENVSTRING, "tok={:?} tokstr={:?}", tok(), tokstr());

        zshlex();
        assert_eq!(tok(), STRING_LEX);
    }

    #[test]
    fn test_array_assignment() {
        let _ = lex_init("arr=(a b c)");
        set_incmdpos(true);
        zshlex();
        assert_eq!(tok(), ENVARRAY);
    }

    #[test]
    fn test_process_substitution() {
        let _ = lex_init("diff <(ls) >(cat)");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
        // <(ls) is tokenized into the string

        zshlex();
        assert_eq!(tok(), STRING_LEX);
        // >(cat) is tokenized
    }

    #[test]
    fn test_arithmetic() {
        let _ = lex_init("echo $((1+2))");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
    }

    #[test]
    fn test_semicolon_variants() {
        let _ = lex_init("case x in a) cmd;; b) cmd;& c) cmd;| esac");

        // Skip to first ;;
        loop {
            zshlex();
            if tok() == DSEMI || tok() == ENDINPUT {
                break;
            }
        }
        assert_eq!(tok(), DSEMI);

        // Find ;&
        loop {
            zshlex();
            if tok() == SEMIAMP || tok() == ENDINPUT {
                break;
            }
        }
        assert_eq!(tok(), SEMIAMP);

        // Find ;|
        loop {
            zshlex();
            if tok() == SEMIBAR || tok() == ENDINPUT {
                break;
            }
        }
        assert_eq!(tok(), SEMIBAR);
    }
}
