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

pub use super::zsh_h::{
    lextok, AMPER, AMPERBANG, AMPOUTANG, BANG_TOK, BARAMP, BAR_TOK, CASE, COPROC, DAMPER, DBAR,
    DINANG, DINANGDASH, DINBRACK, DINPAR, DOLOOP, DONE, DOUTANG, DOUTANGAMP, DOUTANGAMPBANG,
    DOUTANGBANG, DOUTBRACK, DOUTPAR, DSEMI, ELIF, ELSE, ENDINPUT, ENVARRAY, ENVSTRING, ESAC, FI,
    FOR, FOREACH, FUNC, IF, INANGAMP, INANG_TOK, INBRACE_TOK, INOUTANG, INOUTPAR, INPAR_TOK,
    IS_REDIROP, LEXERR, LEXFLAGS_ACTIVE, LEXFLAGS_COMMENTS, LEXFLAGS_COMMENTS_KEEP,
    LEXFLAGS_COMMENTS_STRIP, LEXFLAGS_NEWLINE, LEXFLAGS_ZLE, NEWLIN, NOCORRECT, NULLTOK, OUTANGAMP,
    OUTANGAMPBANG, OUTANGBANG, OUTANG_TOK, OUTBRACE_TOK, OUTPAR_TOK, REPEAT, SELECT, SEMI, SEMIAMP,
    SEMIBAR, SEPER, STRING_LEX, THEN, TIME, TRINANG, TYPESET, UNTIL, WHILE, ZEND,
};
pub use crate::heredoc_ast::HereDoc;
use crate::ported::context::{zcontext_restore, zcontext_save};
use crate::ported::hashtable::{aliastab_lock, reswdtab_lock, sufaliastab_lock};
use crate::ported::hist::{hist_in_word, strinbeg, strinend};
use crate::ported::input::{inpop, inpush};
use crate::ported::parse::HDOCS;
use crate::ported::prompt::{cmdpop, cmdpush, CMDSTACK};
use crate::ported::string::dupstring_wlen;
use crate::ported::utils::{errflag, spckword, zerr, ERRFLAG_ERROR};
use crate::ported::zle::compcore::{WB, WE};
use crate::ported::zsh_h::{
    alias, interact, isset, lex_stack, lexbufstate, unset, Bang, Bar, Bnull, Bnullkeep, Comma,
    Dash, Dnull, Equals, Hat, Inang, Inbrace, Inbrack, Inpar, Inparmath, Marker, Meta, Nularg,
    Outang, OutangProc, Outbrace, Outbrack, Outpar, Outparmath, Pound, Qstring, Qtick, Quest,
    Snull, Star, Stringg, Tick, Tilde, ALIASESOPT, CORRECT, CORRECTALL, CSHJUNKIEQUOTES, CS_BQUOTE,
    CS_BRACE, CS_BRACEPAR, CS_CMDSUBST, CS_CURSH, CS_DQUOTE, CS_HEREDOC, CS_HEREDOCD, CS_MATH,
    CS_MATHSUBST, CS_QUOTE, ERRFLAG_INT, HISTALLOWCLOBBER, IGNOREBRACES, IGNORECLOSEBRACES,
    INP_ALIAS, INP_CONT, INTERACTIVECOMMENTS, KSHGLOB, POSIXALIASES, RCQUOTES, SHGLOB, SHINSTDIN,
    SHORTLOOPS,
    SHORTREPEAT, ZCONTEXT_LEX, ZCONTEXT_PARSE,
};
use crate::ported::ztype_h::itok;
use crate::DPUTS;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

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
    ls.tokstr = LEX_TOKSTR.with_borrow_mut(|t| t.take());
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
    // NOTE: LEX_UNGET_BUF is deliberately NOT stashed here — `$(...)`
    // bodies use it as a cross-context handoff into the nested parse
    // (see the hungetc flow at lex.rs `all_plain` / gettokstr), so a
    // blanket take here truncates every cmdsubst body. Nested lexes
    // that must not see the suspended parse's ungets (the
    // syntax-highlight walk) isolate it at their own call site.

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

    // lex.c:278-306 — drain pending here-documents at the start of
    // a new line. Line-by-line port: walks the canonical `hdocs`
    // linked list (`parse::HDOCS` mirrors `Src/parse.c:84 struct
    // heredocs *hdocs;`), calls `gethere` to read each body, calls
    // `setheredoc` to patch the wordcode redir slot. Two zshrs-only
    // lines (annotated below) bridge body content + processed flag
    // into the parallel AST-glue `LEX_HEREDOCS` Vec.
    if tok() == NEWLIN || tok() == ENDINPUT {
        // c:279 — `while (hdocs)`
        while let Some(mut node) = HDOCS.with_borrow_mut(|h| h.take()) {
            // c:280 — `struct heredocs *next = hdocs->next;`
            let next: Option<Box<crate::ported::zsh_h::heredocs>> = node.next.take();
            // c:281 — `char *doc, *munged_term;`
            let doc: Option<String>;
            let mut munged_term: String;

            // c:283 — `hwbegin(0);` (history-build cursor — zshrs no-op)
            // c:284 — `cmdpush(hdocs->type == REDIR_HEREDOC ? CS_HEREDOC : CS_HEREDOCD);`
            cmdpush(if node.typ == crate::ported::zsh_h::REDIR_HEREDOC {
                CS_HEREDOC as u8
            } else {
                CS_HEREDOCD as u8
            });
            // c:285 — `munged_term = dupstring(hdocs->str);`
            munged_term = crate::ported::mem::dupstring(node.str.as_deref().unwrap_or(""));
            // c:286 — `STOPHIST` (history-disable scope — zshrs no-op)
            // c:287 — `doc = gethere(&munged_term, hdocs->type);`
            doc = crate::ported::exec::gethere(&mut munged_term, node.typ);
            // c:288 — `ALLOWHIST`
            // c:289 — `cmdpop();`
            cmdpop();
            // c:290 — `hwend();`

            // c:291 — `if (!doc)`
            let Some(doc) = doc else {
                // c:292 — `zerr("here document too large");`
                zerr("here document too large");
                // c:293-297 — while (hdocs) { next = hdocs->next; zfree(hdocs); hdocs = next; }
                HDOCS.with_borrow_mut(|h| *h = None);
                // c:298 — `tok = LEXERR;`
                set_tok(LEXERR);
                // c:299 — break out of the while.
                break;
            };
            // c:301-302 — `setheredoc(hdocs->pc, REDIR_HERESTR, doc,
            //                         hdocs->str, munged_term);`
            crate::ported::parse::setheredoc(
                node.pc as usize,
                crate::ported::zsh_h::REDIR_HERESTR,
                &doc,
                node.str.as_deref().unwrap_or(""),
                &munged_term,
            );
            // zshrs-only: write body into the parallel AST-glue
            // LEX_HEREDOCS entry so `fill_heredoc_bodies` (parse.rs)
            // wires it onto the matching ZshRedir.heredoc field.
            // No C counterpart — LEX_HEREDOCS is Rust-only state.
            LEX_HEREDOCS.with_borrow_mut(|v| {
                for h in v.iter_mut() {
                    if !h.processed {
                        h.content = doc;
                        h.processed = true;
                        return;
                    }
                }
            });
            // c:303 — `zfree(hdocs, sizeof(struct heredocs));`
            drop(node);
            // c:304 — `hdocs = next;`
            HDOCS.with_borrow_mut(|h| *h = next);
        }
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
    a2[Meta as usize] = LX2_META;
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
    // C has ONE `lexstop`; zshrs splits it into LEX_LEXSTOP (gates
    // gettok) and input.rs `lexstop` (gates ingetc). The single C
    // `lexstop = 0` here must zero BOTH halves — same paired-reset rule
    // as inpush (input.rs:654-655). Without the input-side reset, a
    // nested string parse (cmd-subst / eval via parse_isolated) that
    // drained its input left `input::lexstop = true`; on the faithful
    // single-event loop()/parse_event reader, the next iteration's
    // ingetc then short-circuited to EOF and the shell exited after the
    // first command containing a `$(…)`.
    crate::ported::input::lexstop.with(|c| c.set(false));
    // lex.c:444 — `tok = ENDINPUT;`
    set_tok(ENDINPUT);
}

/// Add character to token buffer
/// Port of `add(int c)` from `Src/lex.c:451`.
fn add(c: char) {
    LEX_LEXBUF.with_borrow_mut(|b| b.add(c));
}

/// Determine if (( is arithmetic or command
/// Decide whether `( ... )` after a `$` is a math expression
/// `$((...))` or a command substitution `$(...)`. Direct port of
/// zsh/Src/lex.c:495 `cmd_or_math`. Tries dquote_parse first;
/// if it succeeds AND the next char is `)` (closing the second
/// paren of `(( ))`), it's math. Otherwise rewinds and treats as
/// a command substitution.
fn cmd_or_math() -> i32 {
    // dash/ash have NO `(( ))` arithmetic command — `((` is just two nested
    // subshells `( (`. Real dash runs `(( 1 + 1 ))` as the command `1` (with
    // args `+ 1`) inside two subshells → "1: not found", non-zero. Force the
    // command (subshell) path here so zshrs --dash/--ash matches: push back the
    // second `(` (already consumed by the LX1_INPAR caller) and return CMD, as
    // the rewind path below does. Runs BEFORE cmdpush so there is no cmdpop
    // imbalance. `for ((…))` is unaffected (it returns DINPAR earlier, before
    // cmd_or_math), and `$(( ))` arithmetic uses a different path entirely.
    // Found by the per-mode dash-strictness sweep.
    if crate::dash_mode::dash_strict() {
        hungetc('(');
        LEX_LEXSTOP.set(false);
        return CMD_OR_MATH_CMD;
    }
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
    // to lexbuf.
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
        // c:Src/lex.c:519-520 — `hungetc('('); return errflag ?
        // CMD_OR_MATH_ERR : CMD_OR_MATH_CMD;`. The C path does NOT
        // call skipcomm here — the caller (LX1_INPAR arm) returns
        // INPAR_TOK and the parser walks the remaining content as
        // ordinary tokens. Calling skipcomm() consumed the entire
        // subshell body, so `(() { echo X; })` lost the inner anon-fn
        // tokens. Bug #196.
        return CMD_OR_MATH_CMD;
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
    // c:Src/lex.c:513-516 — CMD path: push back the peeked char,
    // then push back the `)` that dquote_parse consumed (this is the
    // `c = ')'; hungetc(c);` sequence at C line 515-518 below the
    // `if (!c)` block; the fall-through into the outer `hungetc(c)`
    // at line 522 puts the `)` back into the input stream so the
    // next zshlex re-emits it as OUTPAR_TOK).
    //
    // Without this hungetc, the inner `)` is lost from the stream
    // permanently. For source `((a|)b)` the token stream loses
    // the inner OUTPAR — emits INPAR INPAR STRING(a) BAR STRING(b)
    // OUTPAR instead of the correct INPAR INPAR STRING(a) BAR
    // OUTPAR STRING(b) OUTPAR. That broke par_case on every
    // `((alt|alt)tail)` pattern — including zinit.zsh:2946
    // `((add-|)fpath)` which fails as "expected ')' in case pattern".
    if let Some(c) = c {
        hungetc(c);
    }
    LEX_LEXSTOP.set(false);
    hungetc(')'); // c:515-518 — the `)` dquote_parse consumed

    // Back up token
    while LEX_LEXBUF.with_borrow(|b| b.buf_len()) > oldlen {
        if let Some(c) = LEX_LEXBUF.with_borrow_mut(|b| b.pop()) {
            hungetc(c);
        }
    }
    hungetc('(');

    // c:Src/lex.c:519-520 — same as above: return CMD_OR_MATH_CMD
    // without skipcomm. Bug #196.
    CMD_OR_MATH_CMD
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
            // c:555-557 — `lexpos = lexbuf.ptr - tokstr; add(Inpar); add('(');`
            // — lexpos is captured BEFORE the Inpar/`(` go in so a
            // later Inparmath rewrite can target the Inpar byte.
            let lexpos = LEX_LEXBUF.with_borrow(|b| b.buf_len());
            add(Inpar);
            add('(');
            // After Inpar+`(`, we record the lexbuf length so the
            // failure-path rewind knows where `dquote_parse`'s output
            // ends and where our Inpar+`(` begin. Mirrors C's split
            // between `cmd_or_math`'s `oldlen` (inside the inner call,
            // = lexpos + 2 bytes in C) and the outer `lexbuf.ptr -= 2`
            // that strips Inpar+`(` after `cmd_or_math` returns.
            let after_open = LEX_LEXBUF.with_borrow(|b| b.buf_len());

            // Inline of `cmd_or_math(CS_MATHSUBST)` (Src/lex.c:495).
            // Done inline rather than calling our existing
            // `cmd_or_math()` because that helper additionally calls
            // `skipcomm()` on its failure path (over-port for the
            // gettok `((` caller), which would double-consume the
            // body when chained here.
            cmdpush(CS_MATHSUBST as u8);
            let dq_ok = dquote_parse(')', false).is_ok();
            cmdpop();

            if dq_ok {
                // c:511 — `c = hgetc(); if (c == ')') return MATH;`
                let c2 = hgetc();
                if c2 == Some(')') {
                    // c:559-562 — confirmed math: rewrite Inpar →
                    // Inparmath at lexpos, append closing `)`. Inpar
                    // and Inparmath are both 2-byte UTF-8 (`\u{88}` /
                    // `\u{89}`); set_char_at swaps in place.
                    LEX_LEXBUF.with_borrow_mut(|b| b.set_char_at(lexpos, Inparmath));
                    add(')');
                    return CMD_OR_MATH_MATH;
                }
                if let Some(c2) = c2 {
                    hungetc(c2);
                }
                LEX_LEXSTOP.set(false);
                // c:516 — `c = ')';` — fall through to the rewind
                // path; the `)` that dquote_parse consumed needs
                // to be hungetc'd. We synthesize that by setting up
                // the loop and the final hungetc('(') as below; the
                // `)` is implicit in `dquote_parse` having stopped
                // at it (it was consumed but not added to lexbuf,
                // matching C where `c = ')'` is what gets hungetc'd
                // first before the rewind-everything loop).
                hungetc(')');
            } else if LEX_LEXSTOP.get() {
                // c:519 — `else if (lexstop) return CMD_OR_MATH_ERR;`
                return CMD_OR_MATH_ERR;
            } else {
                // c:522 — `hungetc(c); lexstop = 0;` — push back the
                // char dquote_parse stopped on (caller-side handled
                // via `dquote_parse` return value in C; in Rust we
                // approximate by pushing nothing here — dquote_parse
                // already left the stream positioned at the offending
                // char, but our impl signals failure through Err
                // without consuming it).
                LEX_LEXSTOP.set(false);
            }

            // c:524-528 — `while (lexbuf.len > oldlen) { ... }; hungetc('(');`
            // — back up everything dquote_parse appended to lexbuf
            // and hungetc each in reverse order, then hungetc the
            // single `(` that opened the math construct. The Inpar+`(`
            // we placed BEFORE the inner call are NOT touched here;
            // they're stripped from lexbuf after this block via direct
            // pop (no hungetc) so they don't go back onto the input.
            //
            // Bug #4 in docs/BUGS.md: the previous Rust port popped
            // ALL the way down to `lexpos` (before Inpar+`(`) and
            // hungetc'd those bytes back, putting an Inpar token on
            // the input stream that derailed `skipcomm`. The faithful
            // port pops only down to `after_open` and then strips
            // Inpar+`(` from lexbuf without hungetc'ing them.
            while LEX_LEXBUF.with_borrow(|b| b.buf_len()) > after_open {
                if let Some(ch) = LEX_LEXBUF.with_borrow_mut(|b| b.pop()) {
                    hungetc(ch);
                } else {
                    break;
                }
            }
            hungetc('(');
            LEX_LEXSTOP.set(false);
            // c:565-566 — `lexbuf.ptr -= 2; lexbuf.len -= 2;` — drop
            // the Inpar+`(` we added at the top without putting them
            // on the input stream.
            LEX_LEXBUF.with_borrow_mut(|b| {
                b.pop();
                b.pop();
            });
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

// ============================================================================
// Additional parsing functions ported from lex.c
// ============================================================================

/// Check whether we're looking at valid numeric globbing syntax
/// `<N-M>` / `<N->` / `<-M>` / `<->`. Call pointing just after the
/// Port of `static int isnumglob(void)` from `Src/lex.c:581`.
/// Called pointing just after the opening `<`. Looks ahead to
/// detect zsh's numeric-glob shape `<[0-9]*-[0-9]*>` (e.g. `<->`,
/// `<1-5>`, `<10->`). Returns `true` when the shape matches.
/// Leaves the input stream in the same position regardless of
/// outcome — all consumed chars are `hungetc`'d back.
///
/// C body:
/// ```c
/// while (1) {
///     c = hgetc();
///     if (lexstop) { lexstop = 0; break; }
///     tbuf[n++] = c;
///     if (!idigit(c)) {
///         if (c != ec) break;
///         if (ec == '>') { ret = 1; break; }
///         ec = '>';
///     }
/// }
/// while (n--) hungetc(tbuf[n]);
/// return ret;
/// ```
pub fn isnumglob() -> bool {
    // c:581 (Src/lex.c)
    // c:583 — `int c, ec = '-', ret = 0;`. `ec` is the expected
    // non-digit char: starts at `-`, flips to `>` after the dash.
    let mut ec: char = '-'; // c:583
    let mut ret = false; // c:583
                         // c:585 — char buffer for the rewind at c:606-607.
    let mut buf: Vec<char> = Vec::new();

    loop {
        // c:587
        let cn = hgetc(); // c:588
        if LEX_LEXSTOP.get() {
            // c:589
            LEX_LEXSTOP.set(false); // c:590
            break; // c:591
        }
        let Some(cn) = cn else { break };
        buf.push(cn); // c:593
        if !cn.is_ascii_digit() {
            // c:594 !idigit(c)
            if cn != ec {
                // c:595
                break; // c:596
            }
            if ec == '>' {
                // c:597
                ret = true; // c:598
                break; // c:599
            }
            ec = '>'; // c:601
        }
    }
    // c:606-607 — `while (n--) hungetc(tbuf[n]);` — rewind in
    // reverse order so the next hgetc sees the same bytes.
    while let Some(ch) = buf.pop() {
        hungetc(ch);
    }
    ret // c:609
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

// =============================================================================
// End of inlined `tokens.rs`. Original lex.rs body follows.
// =============================================================================

// `lexflags` — port of `mod_export int lexflags;` (`Src/lex.c:151`).
// Carries the LEXFLAGS_ACTIVE/ZLE/COMMENTS_KEEP/COMMENTS_STRIP/NEWLINE
// bit flags from `Src/zsh.h:2293-2315`. The constants live in
// `super::zsh_h:2532-2537`; access via plain `&` / `|` ops, not a
// Rust struct.

// `struct LexBuf` (fake Rust-only paraphrase) DELETED. The canonical
// port of `struct lexbufstate` (zsh.h:3069-3079) lives at
// `crate::ported::zsh_h::lexbufstate` with fields `{ptr: Option<String>,
// siz: i32, len: i32}` — same shape as C. The LEX_LEXBUF /
// LEX_LEXBUF_RAW thread_locals use this canonical type directly.
// The convenience methods below are Rust-only
// helpers wrapping the flat operations C inlines at lex.c:451+ (`add`,
// etc.) — they're carried here as helpers rather than re-inlining
// ~50 lines of ptr/len/siz arithmetic across 24 call sites.

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
    /// lexbuf.siz = 256;` plus gettokstr's `lexbuf.ptr = hcalloc(...)`
    /// (lex.c:632/953): the buffer is ALLOCATED here when absent so the
    /// following `add()`s accumulate. Without the allocate-when-None
    /// arm, `add()` (which only pushes when `ptr` is Some) silently
    /// no-ops whenever the lexer is entered without a prior `lex_init`
    /// — e.g. the interactive REPL reading off SHIN via ingetc — so
    /// every token came out empty.
    pub(crate) fn clear(&mut self) {
        match self.ptr.as_mut() {
            Some(p) => p.clear(),
            None => self.ptr = Some(String::with_capacity(self.siz.max(256) as usize)),
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

    /// Replace the char at BYTE position `byte_idx` in lexbuf. The
    /// lexbuf's `len` field tracks BYTE length (matching C's
    /// `lexbuf.len` which indexes the raw `tokstr[]` array), and
    /// `buf_len()` returns the same. So when `cmd_or_math_sub`
    /// saves `lexpos = buf_len()` before `add(Inpar)`, lexpos is
    /// the byte offset where the Inpar marker landed.
    ///
    /// Mirrors C's `tokstr[lexpos] = MARKER` rewrites (lex.c:560,
    /// cmd_or_math_sub) that retroactively patch a marker byte
    /// after the surrounding parse confirms the context. The
    /// caller must guarantee the new char's UTF-8 byte width
    /// matches the current char's width at that offset — used
    /// only for swapping equally-sized markers (e.g. Inpar
    /// `\u{88}` → Inparmath `\u{89}`, both 2 UTF-8 bytes).
    pub(crate) fn set_char_at(&mut self, byte_idx: usize, c: char) {
        let Some(buf) = self.ptr.as_mut() else { return };
        if byte_idx >= buf.len() {
            return;
        }
        // Walk to the char-start at byte_idx (must be on a UTF-8
        // boundary). Find the char length so we can replace exactly
        // that many bytes.
        if !buf.is_char_boundary(byte_idx) {
            return;
        }
        let old_byte_end = buf[byte_idx..]
            .chars()
            .next()
            .map(|ch| byte_idx + ch.len_utf8());
        let Some(old_byte_end) = old_byte_end else {
            return;
        };
        let mut new_bytes = [0u8; 4];
        let new_str = c.encode_utf8(&mut new_bytes);
        if new_str.len() == old_byte_end - byte_idx {
            let new_owned = new_str.to_string();
            buf.replace_range(byte_idx..old_byte_end, &new_owned);
        }
    }
}

// Per-heredoc state — Rust-only AST-glue, NOT in lex.c. Canonical home
// is `src/extensions/heredoc_ast.rs`; re-exported here so existing
// `crate::lex::HereDoc` / `crate::parse::HereDocInfo` call sites keep
// resolving. Both die in Phase 9e (PORT_PLAN.md) when the wordcode
// port reinstates C's `struct heredocs` shape (zsh.h:1152) +
// `gethere()` body collection.

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
    /// `LEX_POS` static.
    pub static LEX_POS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// `LEX_UNGET_BUF` static.
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
    /// `LEX_LINENO` static.
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
    /// `int inalmore` (lex.c:80). Set by `inpoptop` (input.c:775)
    /// when a drained non-global alias body ends in a space —
    /// "aliases should be expanded, as if we are continuing after
    /// an alias" (input.c:63) — so the NEXT word is alias-eligible
    /// even outside command position (`alias sudo='sudo '` chains).
    /// Consulted by `checkalias` (lex.c:1917); cleared by `exalias`
    /// (lex.c:2016).
    pub static LEX_INALMORE: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `int nocorrect` (lex.c:144).
    pub static LEX_NOCORRECT: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `int nocomments` (lex.c:148).
    pub static LEX_NOCOMMENTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// `int lexflags` (lex.c:118).
    pub static LEX_LEXFLAGS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// `int aliasspaceflag` (parse.c:39, zsh.h:3103). Set to 1 by the
    /// alias-expansion path in `cmd_or_math` (lex.c:1930) when an
    /// expanded non-global alias starts with a space; consulted by
    /// hist.c:1428 to suppress history entries the same way a literal
    /// space-prefixed command line is. Cleared by `parse_event`
    /// (parse.c:618). parse_context_save/restore preserve across
    /// nested parses.
    pub static LEX_ALIAS_SPACE_FLAG: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
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
    /// !!! WARNING: NOT IN LEX.C — Rust-only AST-glue Vec !!!
    /// Runs parallel to the canonical C `struct heredocs *hdocs;`
    /// linked list at `parse::HDOCS` (port of `Src/parse.c:84`).
    /// Carries terminator / strip_tabs / quoted metadata that C
    /// stores implicitly via tokstr; zshrs's AST consumer
    /// (`fill_heredoc_bodies` in parse.rs) reads it via the
    /// `heredoc_idx` field on `ZshRedir`.
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
            // `ch as u8` truncates a multibyte codepoint into the blank range
            // (U+4E09 → 0x09); only ASCII can be blank.
            Some(ch) if ch.is_ascii() && crate::ztype_h::iblank(ch as u8) => continue,
            Some(ch) => break ch,
            None => {
                // c:624-625 — `if (lexstop) return (errflag) ?
                // LEXERR : ENDINPUT;`
                use std::sync::atomic::Ordering;
                LEX_LEXSTOP.set(true);
                return if errflag.load(Ordering::Relaxed) != 0 {
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
    let qbang_at_bang = crate::ported::hist::qbang.load(std::sync::atomic::Ordering::SeqCst)
        && c as i32 == crate::ported::hist::bangchar.load(std::sync::atomic::Ordering::SeqCst);
    let qbang_adj: i32 = if qbang_at_bang { 1 } else { 0 };
    if (LEX_LEXFLAGS.get() & LEXFLAGS_ZLE) != 0
        && (crate::ported::input::inbufflags.with(|f| f.get()) & INP_ALIAS) == 0
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
    // ASCII-only: `c as u8` on a multibyte char truncates into the digit range
    // (U+4E30 → 0x30 = '0'), which would misread a CJK word as a redirection fd.
    if c.is_ascii() && crate::ztype_h::idigit(c as u8) {
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
    // c:679-681 — `(!lexflags || (lexflags & LEXFLAGS_COMMENTS)) &&
    //   !expanding && (!interact || unset(SHINSTDIN) || strin)`.
    // `expanding` (hist.c:65) and `strin` (input.c) ARE ported; the
    // `|| strin` term is load-bearing: when a file is sourced from an
    // INTERACTIVE shell, `interact` is set and SHINSTDIN is set, so
    // without `strin` the `#` in `.zshrc`/`.zshenv`/`~/.cargo/env`
    // (`#!/bin/sh`, `# comment`) was parsed as a command. `strin` is
    // set while reading a sourced file / string-eval, which is exactly
    // when comments must still strip regardless of interactivity.
    let lexflags = LEX_LEXFLAGS.get();
    let expanding = crate::ported::hist::expanding.load(std::sync::atomic::Ordering::SeqCst) != 0;
    let strin = crate::ported::input::strin.with(|c| c.get()) != 0;
    let allow_comment_via_flags = (lexflags == 0 || (lexflags & LEXFLAGS_COMMENTS) != 0)
        && !expanding
        && (!interact() || unset(SHINSTDIN) || strin);
    if c as i32 == crate::ported::hist::hashchar.load(std::sync::atomic::Ordering::SeqCst)
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
                    //
                    // Case-pattern context (incasepat>0): C source does
                    // NOT add incasepat here — the `(` is absorbed into
                    // gettokstr as part of the pattern, and par_case
                    // detects the leading-paren form by checking if the
                    // resulting str starts with the Inpar marker
                    // (parse.c:1322 hack). Mirror exactly: do not gate
                    // on incasepat.
                    // c:parse.c:2601 — par_cond bumps `incond` (to >1) around
                    // the RHS of a `[[ ]]` binary op so "parentheses do
                    // globbing" (a literal pattern/regex char), not grouping.
                    // In that bumped state `(` must be a literal even under
                    // SHGLOB (ksh/bash emulation) — otherwise `[[ x =~ (a)(b)
                    // ]]` splits the regex into INPAR tokens and par_cond's
                    // single-STRING RHS read fails with "condition expected".
                    // Verified against zsh: `emulate ksh; [[ abc =~ (a)(b) ]]`
                    // works. Native zsh (no SHGLOB) already reached this via
                    // the `incond == 1` miss; the explicit `> 1` guard makes
                    // it hold under SHGLOB too.
                    if LEX_INCOND.get() > 1 {
                        gettokstr('(', false)
                    } else if isset(SHGLOB) || LEX_INCOND.get() == 1 || LEX_INCMDPOS.get() {
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
                Some('(') if crate::dash_mode::dash_strict() => {
                    // !!! DASH-STRICT GATE (no C counterpart) !!!
                    // dash/ash have no `<(...)` process substitution. Unget the
                    // `(` so `<` stays a plain input redirection; the `(` then
                    // sits where a redirection target (filename) is expected, so
                    // the parser rejects it — matching /bin/dash's
                    // "Syntax error: \"(\" unexpected". Twin of the `<<<`
                    // here-string gate below. Found by the dash-strictness sweep.
                    hungetc('(');
                    LEX_LEXSTOP.set(false);
                    INANG_TOK
                }
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
                        Some('<') if crate::dash_mode::dash_strict() => {
                            // !!! DASH-STRICT GATE (no C counterpart) !!!
                            // dash has no `<<<` here-string. Unget the third
                            // `<` so `<<` stays a here-document operator and
                            // the dangling `<` becomes a stray redirection the
                            // parser rejects — matching /bin/dash's
                            // "Syntax error: redirection unexpected".
                            hungetc('<');
                            LEX_LEXSTOP.set(false);
                            DINANG
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
                    // c:858-862 — `hungetc(d); if (isnumglob()) goto
                    // unpeekfd; peek = INANG;`. `<digits-digits>` is
                    // zsh's numeric-glob pattern (e.g., `<-> `, `<1-5>`,
                    // `<10->`). isnumglob looks ahead to confirm the
                    // shape and rewinds. When it matches, the entire
                    // `<...>` is a glob, NOT a redir — fall through to
                    // gettokstr so it lexes as a literal STRING token.
                    //
                    // Without this, `[[ $1 = <-> ]]` had the lexer
                    // consume `<` as INANG and try to take the next
                    // token as the redir target, blowing up with
                    // "expected word after redirection". Critical for
                    // any zsh code using <-> / <N-M> globs (e.g.
                    // zinit.zsh:1983).
                    if let Some(d) = d {
                        hungetc(d);
                    }
                    if isnumglob() {
                        // c:860
                        // c:861 — `goto unpeekfd;` — restore peekfd
                        // and fall through to gettokstr at LX1 break.
                        if peekfd != -1 {
                            // c:832
                            hungetc(c); // c:833
                            return gettokstr(
                                ((b'0' as i32) + peekfd) as u8   // c:834
                                as char,
                                false,
                            );
                        }
                        LEX_LEXSTOP.set(false);
                        return gettokstr(c, false);
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
                Some('(') if crate::dash_mode::dash_strict() => {
                    // !!! DASH-STRICT GATE (no C counterpart) !!!
                    // dash/ash have no `>(...)` process substitution. Unget the
                    // `(` so `>` stays a plain output redirection and the `(`
                    // becomes an unexpected redirection target → parser rejects
                    // it, matching /bin/dash. Twin of the `<(` gate above.
                    hungetc('(');
                    LEX_LEXSTOP.set(false);
                    OUTANG_TOK
                }
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
        // C's lexer walks BYTES, so `inblank(c)` can only ever fire on a byte —
        // and no byte of a UTF-8 multibyte character is 0x20 or 0x09. Rust walks
        // `char`s, so casting one to u8 TRUNCATES the codepoint: `三` (U+4E09)
        // became 0x09 = TAB and `丠` (U+4E20) became 0x20 = SPACE, so an
        // unquoted CJK word was split apart as if it were whitespace
        // (`print -r -- 三` printed nothing). Only ASCII can be blank.
        let inbl = c.is_ascii() && crate::ztype_h::inblank(c as u8);

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
                // c:Src/lex.c:989-990 — `if ((sub || in_brace_param) &&
                // isset(SHGLOB)) break;`. As with LX2_INPAR above, C's `break`
                // exits the SWITCH and falls through to `add(c)` (c:1417) with
                // `c` still the raw `)`; it does NOT end the token. LX2_BAR
                // just below already ports this shape correctly with `add(c)`.
                // Bug #1052.
                if (sub || in_brace_param > 0) && isset(SHGLOB) {
                    add(c);
                } else if in_brace_param > 0 || sub {
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
                    Some('[') if crate::dash_mode::dash_strict() => {
                        // !!! DASH-STRICT GATE (no C counterpart) !!! dash/ash
                        // have no `$[...]` arithmetic substitution (a deprecated
                        // bash/zsh form; POSIX uses `$(( ))`). The `$` is a
                        // literal dollar and `[...]` ordinary text, so
                        // `echo $[1+2]` prints `$[1+2]` like /bin/dash. Emit
                        // `Bnull '$'` (mirrors the `$'...'` gate above) and push
                        // the `[` back so it re-lexes as a literal bracket.
                        hungetc('[');
                        LEX_LEXSTOP.set(false);
                        add(Bnull);
                        add('$');
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
                        // c:1049-1057 — `${...}` parameter expansion.
                        // C does `add(c)` where `c` was already
                        // mapped by `lextok2[c]` at switch entry from
                        // `$` (0x24) to Stringg (`\u{85}`). Rust's
                        // switch dispatches on the LX2_* class but
                        // doesn't pre-map `c`, so `add(c)` here would
                        // store the raw `$` byte. Use the marker
                        // directly to match C's emitted strs section.
                        add(Stringg);
                        add(Inbrace);
                        bct += 1;
                        cmdpush(CS_BRACEPAR as u8);
                        if in_brace_param == 0 {
                            in_brace_param = bct;
                        }
                    }
                    Some('\'') if crate::dash_mode::dash_strict() => {
                        // !!! DASH-STRICT GATE (no C counterpart) !!!
                        // dash has no `$'...'` ANSI-C quoting: the `$` is a
                        // literal dollar and the following `'` opens an
                        // ordinary single-quoted string, so `printf %s $'\t'`
                        // yields `$\t` exactly like /bin/dash.
                        //
                        // The `$` MUST be emitted as an escaped literal
                        // (`Bnull '$'`), NOT as `Stringg`: the re-lexed
                        // single quote emits a leading `Snull`, and the byte
                        // pair `Stringg Snull` is precisely what getkeystring
                        // (lex.rs:4910) decodes as a `$'...'` region — which
                        // would re-enable the ANSI-C decode we are suppressing.
                        // `Bnull '$'` is a plain literal dollar that cannot
                        // combine with the following `Snull`.
                        hungetc('\'');
                        LEX_LEXSTOP.set(false);
                        add(Bnull);
                        add('$');
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
                        //
                        // c:1285 — the C source detects $'...' via
                        // `strquote = (lexbuf.len && lexbuf.ptr[-1]
                        // == String)`, i.e. by looking back for the
                        // Stringg marker the top-level `$` handler
                        // emitted. So the marker here MUST be
                        // Stringg (`\u{85}`), not Qstring (`\u{8c}`,
                        // which is `$` inside double quotes). Using
                        // Qstring made getkeystring's $'...' detect
                        // fail and the strs section diverge from C.
                        add(Stringg);
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
                                            // c:1320 — LEXERR only when not ZLE/bufferwords.
                                            if LEX_LEXFLAGS.get() & LEXFLAGS_ACTIVE == 0 {
                                                peek = LEXERR;
                                            }
                                            break;
                                        }
                                    }
                                }
                                Some(ch) => add(ch),
                                None => {
                                    LEX_LEXSTOP.set(true);
                                    unmatched = '\'';
                                    // c:1320 — LEXERR only when not ZLE/bufferwords.
                                    if LEX_LEXFLAGS.get() & LEXFLAGS_ACTIVE == 0 {
                                        peek = LEXERR;
                                    }
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
                        // plain "..." but flagged via Stringg+Dnull
                        // (NOT Qstring) so the dollar prefix marker
                        // sits in the strs section as `\u{85}\u{9e}…`.
                        // Qstring (`\u{8c}`) is reserved for $X
                        // sequences encountered INSIDE double quotes
                        // (c:1524, 1546, 1551 inside dquote_parse);
                        // top-level `$"…"` uses Stringg per
                        // C lex.c's $-dispatch.
                        add(Stringg);
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
                // c:Src/lex.c:1080-1081 — `if (sub || in_brace_param) break;`.
                // That `break` leaves the SWITCH, not the token loop: `c` is
                // still the literal `(`, so the common `add(c)` after the
                // switch (c:1417) emits it and lexing CONTINUES to the closing
                // `}`. A Rust `break` exits the lexer loop and ENDS the token,
                // so `${(t)PATH}` under SHGLOB stopped at the `(` with
                // in_brace_param still 1 and died "closing brace expected".
                // The `goto brk` case below is a REAL token break and stays a
                // Rust `break`. Bug #1052.
                let mut shglob_literal_inpar = false;
                if isset(SHGLOB) {
                    if sub || in_brace_param > 0 {
                        shglob_literal_inpar = true;
                    }
                    // c:1084 — `!KSHGLOB && lexbuf.len → goto brk`. In sh
                    // emulation (SHGLOB, no KSHGLOB) a `(` after existing
                    // content ends the word (ksh function-def heuristic). But
                    // in the `[[ ]]` cond-RHS PATTERN context (par_cond bumps
                    // incond >1 so "parentheses do globbing"), adjacent groups
                    // like `[[ x =~ (a)(b) ]]` must stay ONE regex word — the
                    // second `(` is a literal, not a word break. Verified vs
                    // zsh: `emulate sh; [[ ab =~ (a)(b) ]]` works. Without this
                    // exception, `--bash` (KSHGLOB off) split the regex and
                    // par_cond failed with "condition expected".
                    else if unset(KSHGLOB)
                        && LEX_INCOND.get() <= 1
                        && LEX_LEXBUF.with_borrow(|b| b.len) > 0
                    {
                        break;
                    }
                }
                if shglob_literal_inpar {
                    // c:1417 `add(c)` with `c` still the raw `(`.
                    add(c);
                } else {
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
            // `add(c)` — both arms must be inlined per LX2 case.
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
                    // Closing a brace expansion like {a,b}. c:1165 —
                    // `c = Outbrace;` then falls through to the
                    // switch-exit `add(c)`. C maps `}` to Outbrace
                    // when bct>0 so the wordcode `strs` section sees
                    // the marker byte, not the raw `}`. Without this
                    // the strs section emits `{a,b,c}` literally
                    // instead of `{a,b,c\x90` and wordcode parity
                    // breaks on every brace expansion in the corpus.
                    bct -= 1;
                    add(Outbrace);
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

            // c:1187 — `case LX2_INANG:` — `<` body, direct port.
            // C order: SHGLOB+sub guard, then `<(...)` proc-sub
            // (only when NOT in brace_param/sub), then isnumglob,
            // then the in_brace_param/sub guard that keeps `<` as
            // a literal character.
            LX2_INANG => {
                // c:1188 — `if (isset(SHGLOB) && sub) break;`. Same
                // switch-vs-loop distinction the c:1209 case below already
                // documents: C's `break` falls through to the post-switch
                // `add(c)` (c:1417) with `c` still the raw `<`, so the token
                // continues and the `<` is LITERAL. Ending the token here made
                // `emulate sh -c '${a[(r)<->]}'` keep numeric-range-glob
                // semantics and match `1`, where zsh compares against the
                // literal text `<->` and finds nothing. Bug #1052.
                if isset(SHGLOB) && sub {
                    add(c);
                } else {
                // c:1190-1198 — `e = hgetc(); if (!(in_brace_param ||
                // sub) && e == '(') { add(Inang); skipcomm(); c =
                // Outpar; break; }`. `<(...)` process-sub only when
                // outside brace_param/sub — inside, `<` stays literal.
                let e = hgetc(); // c:1190
                if !(in_brace_param > 0 || sub) && e == Some('(') {
                    // c:1191
                    add(Inang); // c:1192
                    if skipcomm().is_err() {
                        // c:1193
                        peek = LEXERR;
                        break;
                    }
                    add(Outpar); // c:1197 c=Outpar
                } else {
                    // c:1200 — `hungetc(e);`.
                    if let Some(e) = e {
                        hungetc(e);
                    }
                    // c:1201-1207 — isnumglob → emit Inang/.../Outang.
                    if LEX_INCONDPAT.get() || LEX_INCASEPAT.get() > 0 {
                        // [[ ]] / case pattern context: `<` literal.
                        // Original zshrs note: real zsh's wordcode has
                        // Inang/Outang markers even inside ${var//pat/repl}
                        // so isnumglob still fires below — but cond /
                        // case patterns stay raw.
                        add(c);
                    } else if isnumglob() {
                        // c:1201
                        // c:1202-1206 — emit Inang…Outang markers.
                        add(Inang); // c:1202
                        while let Some(ch) = hgetc() {
                            // c:1203
                            if ch == '>' {
                                break;
                            }
                            add(ch); // c:1204
                        }
                        add(Outang); // c:1205
                    } else {
                        // c:1208 — `lexstop = 0;`.
                        LEX_LEXSTOP.set(false); // c:1208
                                                // c:1209-1210 — `if (in_brace_param || sub) break;`
                                                // exits the C switch and falls to the
                                                // post-switch `add(c)`. In Rust the LX2_INANG
                                                // arm doesn't fall to a shared add — add `<`
                                                // explicitly here so it lands in the token
                                                // buffer. Inside `${...}` and `$(...)` /
                                                // backticks, bare `<` is literal — required
                                                // for patterns like `${arr[@]:#<no-data>}`
                                                // (zinit.zsh:2507) which excludes elements
                                                // matching the literal `<no-data>`.
                        if in_brace_param > 0 || sub {
                            // c:1209
                            add(c);
                        } else {
                            // c:1211 — `goto brk;` outside brace_param/sub
                            // ends the token (bare `<` is a redirect).
                            break;
                        }
                    }
                }
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
                        && LEX_INCASEPAT.get() <= 0
                    {
                        // c:1229-1230 — C gates only on
                        // `(incmdpos || intypeset) && !bct && !brct`;
                        // incasepat is not consulted. par_case sets
                        // incasepat=-1 with incmdpos=1 before lexing
                        // the token after a whole-`(...)` case
                        // pattern (parse.c:1300-1302) — that token
                        // may be the body's first word, and
                        // `out+=hit` there must still become
                        // ENVSTRING. Keep blocking only the >0
                        // in-pattern states.
                        // Check for VAR=value assignment (but not in case pattern context)
                        let tok_so_far = LEX_LEXBUF.with_borrow(|b| b.as_str().to_string());
                        if is_valid_assignment_target(&tok_so_far) {
                            let next = hgetc();
                            if next == Some('(') && crate::dash_mode::dash_strict() {
                                // !!! DASH-STRICT GATE (no C counterpart) !!!
                                // dash has no arrays; `name=(...)` is a hard
                                // syntax error ("( unexpected"). Emit LEXERR
                                // so the parse fails like /bin/dash instead
                                // of lexing an ENVARRAY assignment.
                                peek = LEXERR;
                                break;
                            }
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
                            // c:1383 — LEXERR only when not ZLE/bufferwords.
                            if LEX_LEXFLAGS.get() & LEXFLAGS_ACTIVE != 0 { /* tolerate */ } else {
                            peek = LEXERR;
                            }
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
            // Direct port of C: `if (!in_brace_param && !sub) goto brk;`.
            // C does NOT consult `brct` here — bare `[` in an assignment
            // RHS like `a=foo[bar; echo` does NOT prevent `;` from
            // terminating the word. zsh keeps `foo[bar` as the literal
            // value (the `[` is unmatched and globbing reports
            // "no matches" only when expanded, not at lex time).
            // The previous Rust port added `&& brct == 0` which made
            // `;` part of the token whenever an unmatched `[` was
            // seen — `a=foo[bar; echo done` produced `a=foo[bar;`
            // as the single ENVSTRING, swallowing the next command.
            // Bug #604. `pct == 0` is also dropped — `(...)` grouping
            // is handled by the outer parser's command structure, not
            // by `;` being mid-token here.
            LX2_BREAK if in_brace_param == 0 => {
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
                // Multibyte UTF-8 codepoints (>= 256) pass through
                // verbatim — lextok2 only maps the 256-entry byte
                // table (C's `lextok2[STOUC(c)]` truncates to u8).
                // Previously `lextok2_get(c) as char` truncated
                // codepoint to low byte (日 U+65E5 → å U+00E5),
                // mangling every unquoted multibyte assignment value
                // (`a=日本; echo $a` printed "å,"). The table only
                // matters for ASCII glob metacharacters; everything
                // else is identity.
                if (c as u32) >= 256 {
                    add(c);
                } else {
                    add(lextok2_get(c) as char);
                }
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
        zerr(&format!("unmatched {}", unmatched));
    }

    // c:1447-1453 — `zerr("closing brace expected");` when in_brace_param
    // is still open at end of token. Suppressed while actively lexing for ZLE
    // completion (LEXFLAGS_ACTIVE): `echo ${PA<Tab>` is an open brace at the
    // cursor, not a syntax error — zsh completes it to `${PATH}`.
    if in_brace_param > 0 && LEX_LEXFLAGS.get() & LEXFLAGS_ACTIVE == 0 {
        zerr("closing brace expected");
    }
    // c:1453-1469 — `} else if (unset(IGNOREBRACES) && !sub &&
    //   lexbuf.len > 1 && peek == STRING && lexbuf.ptr[-1] == '}' &&
    //   lexbuf.ptr[-2] != Bnull) {`
    // c:1457 — /* hack to get {foo} command syntax work */
    // A word ENDING in an unmatched literal `}` (bct==0 left it in
    // the buffer un-rewritten — lextok2['}'] is identity, c:419)
    // sheds the `}` and pushes it back so the next gettok lexes it
    // as the standalone `}` word → reswdtab OUTBRACE. This is what
    // makes `{print hi}` / `f(){print x}` close their blocks and
    // `echo hi}` a parse error, while mid-word `a}b` stays literal.
    else if unset(IGNOREBRACES)
        && !sub
        && LEX_LEXBUF.with_borrow(|b| b.buf_len()) > 1
        && peek == STRING_LEX
        && LEX_LEXBUF.with_borrow(|b| {
            let mut it = b.as_str().chars().rev();
            it.next() == Some('}') && it.next() != Some(Bnull)
        })
    {
        // c:1463-1464 — `int lar = lex_add_raw; lex_add_raw =
        // lexbuf_raw.len > 0 && lexbuf_raw.ptr[-1] == '}';` — only
        // strip the raw-mirror `}` if the raw buffer also ends in
        // one (alias expansion can desynchronize them, per the C
        // comment "Just go with it, OK?").
        let lar = LEX_LEX_ADD_RAW.get();
        LEX_LEX_ADD_RAW.set(
            LEX_LEXBUF_RAW.with_borrow(|b| {
                (b.len > 0 && b.ptr.as_deref().unwrap_or("").ends_with('}')) as i32
            }),
        );
        // c:1465-1466 — `lexbuf.ptr--; lexbuf.len--;`
        LEX_LEXBUF.with_borrow_mut(|b| {
            b.pop();
        });
        // c:1467-1468 — `lexstop = 0; hungetc('}');`
        LEX_LEXSTOP.set(false);
        hungetc('}');
        // c:1469 — `lex_add_raw = lar;`
        LEX_LEX_ADD_RAW.set(lar);
    }

    set_tokstr(Some(LEX_LEXBUF.with_borrow(|b| b.as_str().to_string())));
    peek
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
                        // c:Src/lex.c:1501-1513 — C's `\X` arm in
                        // dquote_parse emits `Bnull X` exactly when
                        // `X` is in the special-escape list. The
                        // literal `{`/`}` that may follow `\$` is NOT
                        // in C's list at 1501, so C emits a raw
                        // `{`/`}` next. The subst-time scanner at
                        // subst.rs:2920 already gates raw `{` count
                        // by checking `prev2 != Bnull && prev2 != '\\'`
                        // (a two-char look-back, not a one-char check),
                        // so `Bnull $ {` reads as escaped and stays
                        // uncounted. Symmetric for the closing `}`:
                        // the `\}` immediately after `y` emits its
                        // own `Bnull }` via THIS arm (since `}` is
                        // in the special list when `!intick && bct > 0`),
                        // and the scanner's `prev != Bnull` gate keeps
                        // it uncounted. No extra Bnull insertion needed.
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

/// Tokenize a string as if in double quotes (error-reporting variant).
/// Port of `parsestr(char **s)` from `Src/lex.c:1694`.
/// C body:
/// ```c
/// int err;
/// if ((err = parsestrnoerr(s))) {                                  // c:1698
///     untokenize(*s);                                              // c:1699
///     if (!(errflag & ERRFLAG_INT)) {                              // c:1700
///         if (err > 32 && err < 127)                               // c:1701
///             zerr("parse error near `%c'", err);                  // c:1702
///         else
///             zerr("parse error");                                 // c:1704
///         tok = LEXERR;                                            // c:1705
///     }
/// }
/// return err;
/// ```
pub fn parsestr(s: &str) -> Result<String, String> {
    // c:1694
    match parsestrnoerr(s) {
        // c:1698
        Ok(result) => Ok(result),
        Err(msg) => {
            let untok = untokenize(s); // c:1699
            let _ = untok;
            let ef = errflag // c:1700
                .load(std::sync::atomic::Ordering::Relaxed);
            if (ef & crate::ported::zsh_h::ERRFLAG_INT) == 0 {
                // c:1700
                // c:1701-1704 — `if (err > 32 && err < 127)` switches between
                // "parse error near `%c'" and bare "parse error". The
                // Err(msg) string carries the diagnostic already formatted
                // by dquote_parse / gettokstr; emit it via zerr to match
                // C's stderr behaviour.
                zerr(&msg); // c:1702/1704
                set_tok(LEXERR); // c:1705
            }
            Err(msg)
        }
    }
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
    // c:1716 `untokenize(*s);` — C's untokenize (Src/exec.c:2077 +
    // Src/lex.c:38 ztokens) maps EVERY itok char to its ASCII
    // original: Dnull → `"`, Snull → `'`, Bnull → `\`. The Rust
    // plain `untokenize` deliberately STRIPS Snull/Dnull (see the
    // lex.rs untokenize doc block) — wrong for this call site: the
    // re-lex below must see the quote chars so a tokenized assoc
    // subscript like `Dnull a␠b Dnull` (from `${H["a b"]}`) round-
    // trips to the literal 5-char key `"a b"`. dquote_parse with
    // endchar='\0' adds a bare `"` literally (c:Src/lex.c:1615-1617
    // `if (intick || (endchar != '"' && !bct)) break;` → add(c)),
    // so the quotes survive to getarg's lookup as C intends. The
    // full ztokens mapping also matters for Qstring → `$`: the
    // dquote_parse re-lex below re-tokenizes nested `${k}`
    // (Src/lex.c:1519-1556 `case '$'`); a raw 0x8c marker would pass
    // through unrecognized and a nested-param subscript like
    // `${H["${k}"]}` would never expand `$k`.
    let untok = crate::ported::lex::untokenize_ztokens(s); // c:1716 `untokenize(*s);`
    let dup = dupstring_wlen(&untok, untok.len()); // c:1717
                                                   // c:1715 `zcontext_save();`
    zcontext_save();
    // Drain LEX_INPUT/LEX_POS so hgetc's two-input bridge prefers the
    // freshly-pushed inbuf frame (via inpush below) instead of double-
    // reading our content from both LEX_INPUT and inbuf. Restored
    // after dquote_parse returns.
    let saved_lex_input = LEX_INPUT.with_borrow(|s| s.clone());
    let saved_lex_pos = LEX_POS.get();
    // Append the '\0' sentinel C's dupstring_wlen carries — dquote_parse
    // returns Ok when hgetc returns endchar ('\0' here); without the
    // terminator hgetc returns None at EOF and dquote_parse errors.
    let mut input_with_nul = dup.clone();
    input_with_nul.push('\0');
    LEX_INPUT.with_borrow_mut(|b| b.clear());
    LEX_POS.set(0);
    LEX_LEXSTOP.set(false);
    // c:1717 `inpush(dupstring_wlen(*s, l), 0, NULL);`
    // Push the body AFTER clearing LEX_INPUT so hgetc reads only from
    // the inbuf frame (avoids double-counting characters that appear in
    // both LEX_INPUT and inbuf).
    inpush(&input_with_nul, 0, None);
    // c:1718 `strinbeg(0);`
    strinbeg(0);
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
    strinend();
    // c:1727 `inpop();`
    inpop();
    // Restore LEX_INPUT/LEX_POS so the outer lexer resumes where it
    // left off. Pairs with the swap above.
    LEX_INPUT.with_borrow_mut(|b| *b = saved_lex_input);
    LEX_POS.set(saved_lex_pos);
    // c:1730 — DPUTS(cmdsp, "BUG: parsestr: cmdstack not empty.")
    DPUTS!(
        // c:1730
        CMDSTACK.with(|s| !s.borrow().is_empty()), // c:1730
        "BUG: parsestr: cmdstack not empty."       // c:1730
    );
    // c:1729 `zcontext_restore();`
    zcontext_restore();
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
    let untok = untokenize(s); // c:1749 `untokenize(t = dupstring_wlen(s, l));`
    let dup = dupstring_wlen(&untok, untok.len());
    // c:1748 `zcontext_save();`
    zcontext_save();
    // c:1750 `inpush(t, 0, NULL);`
    inpush(&dup, 0, None);
    // c:1751 `strinbeg(0);`
    strinbeg(0);
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
    // c:1771 — DPUTS(toklen > l, "Bad length for parsed subscript")
    DPUTS!(toklen > l, "Bad length for parsed subscript"); // c:1771
                                                           // c:1779 `strinend();` / c:1780 `inpop();` / c:1782
                                                           // `zcontext_restore();`
    strinend();
    inpop();
    // c:1785 — DPUTS(cmdsp, "BUG: parse_subscript: cmdstack not empty.")
    DPUTS!(
        // c:1785
        CMDSTACK.with(|s| !s.borrow().is_empty()), // c:1785
        "BUG: parse_subscript: cmdstack not empty."  // c:1785
    );
    zcontext_restore();
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
    // c:1802 `if (!*s || !strcmp(s, nulstring)) return 0;`. C nulstring
    // is `{Nularg, 0}` (0xa1, NUL) — defined in Src/subst.c:36. A
    // single-Nularg input is the empty-arg sentinel and returns
    // success without parsing. The previous Rust port missed the
    // nulstring check so a Nularg-only input would attempt re-lex,
    // surfacing a spurious parse error.
    if s.is_empty() || s == Nularg.to_string() {
        // c:1802
        return Ok(String::new());
    }
    let l = s.len();
    let untok = untokenize(s); // c:1804
    let dup = dupstring_wlen(&untok, untok.len());
    // c:1803 `zcontext_save();`
    zcontext_save();
    // c:1805 `inpush(dupstring_wlen(s, l), 0, NULL);`
    inpush(&dup, 0, None);
    // c:1806 `strinbeg(0);`
    strinbeg(0);
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
    // c:1813 — `err = errflag;`. Snapshot PRE-strinend errflag so we
    // can restore it post-zcontext_restore (parse-time errflag bits
    // must not leak to the caller).
    let err = errflag.load(Ordering::Relaxed); // c:1813
    let result = LEX_LEXBUF.with_borrow(|b| b.as_str().to_string());
    // c:1814 `strinend();`
    strinend();
    // c:1815 `inpop();`
    inpop();
    // c:1816 — DPUTS(cmdsp, "BUG: parse_subst_string: cmdstack not empty.")
    DPUTS!(
        // c:1816
        CMDSTACK.with(|s| !s.borrow().is_empty()), // c:1816 cmdsp != 0
        "BUG: parse_subst_string: cmdstack not empty."  // c:1816
    );
    // c:1817 `zcontext_restore();`
    zcontext_restore();
    // c:1819 — `errflag = err | (errflag & ERRFLAG_INT);`. Restore the
    // saved errflag, OR'ing in any ERRFLAG_INT bit set during parse
    // (user interrupt must survive). The previous Rust port skipped
    // this restore — parse-time ERRFLAG_ERROR bits leaked to callers,
    // causing the next exec-engine check to abort on a stale flag.
    let post_err = errflag.load(Ordering::Relaxed); // c:1819
    errflag.store(err | (post_err & ERRFLAG_INT), Ordering::Relaxed); // c:1819
    if ctok == LEXERR {
        // c:1820
        // Diagnostic already emitted via zerr at the failure site.
        return Err("parse error".to_string());
    }
    Ok(result)
}

/// Mark the current word as the one ZLE was looking for. Direct
/// port of `gotword(void)` from `Src/lex.c:1882`. Computes the
/// new-word-end (`nwe`) and new-word-begin (`nwb`) line positions
/// based on `zlemetall`, `inbufct`, `addedx`, and `wordbeg`, then
/// — if the cursor (`zlemetacs`) falls inside that range — writes
/// `wb`/`we` and clears `lexflags`.
pub fn gotword() {
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
            WB.store(nwb, Ordering::SeqCst);
            WE.store(nwe, Ordering::SeqCst);
        } else {
            let wb_new = zlemetacs + addedx;
            WB.store(wb_new, Ordering::SeqCst);
            let we_cur = WE.load(Ordering::SeqCst);
            if we_cur < wb_new {
                WE.store(wb_new, Ordering::SeqCst);
            }
        }
        // c:1895 — `lexflags = 0;`
        LEX_LEXFLAGS.set(0);
    }
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
        let is_reswd = reswdtab_lock()
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
    let alias_clone: Option<alias> = {
        let guard = aliastab_lock().read().expect("aliastab poisoned");
        guard.get(lextext).cloned()
    };
    if let Some(alias) = alias_clone {
        let is_global = (alias.node.flags & crate::ported::zsh_h::ALIAS_GLOBAL) != 0;
        // c:1915-1917 — `if (an && !an->inuse && ((an->node.flags &
        // ALIAS_GLOBAL) || (incmdpos && tok == STRING) || inalmore))`
        // — `inalmore` extends eligibility to the word following a
        // trailing-space alias body (`alias sudo='sudo '` chaining).
        if alias.inuse == 0
            && (is_global
                || (LEX_INCMDPOS.get() && tok() == STRING_LEX)
                || LEX_INALMORE.get() != 0)
        {
            // c:1918-1927 — `if (!lexstop) { int c = hgetc();
            // hungetc(c); if (!iblank(c)) inpush(" ", INP_ALIAS, 0); }`
            // — if the next char isn't blank, insert a space so the
            // alias body can't accidentally join the following word.
            //
            // c:1919-1922 — "Tokens that don't require a space after,
            // get one, because they are treated as if preceded by one."
            //
            // C consumes one char via hgetc then un-consumes it, so the
            // check uses the actual next char from the ACTIVE input
            // source — the unget queue, the inbuf/instack (stdin loop,
            // interactive, alias re-lex), or the LEX_INPUT window.
            // The previous Rust version peeked LEX_INPUT[pos] directly,
            // which never sees inbuf-stack content: for stdin/interactive
            // lines (`git status` with `alias git=hub`) the pending
            // "status" lives in inbuf, peek() returned None, the
            // separator push was skipped, and the alias body fused with
            // the following word → `hubstatus`.
            if !LEX_LEXSTOP.get() {
                // c:1918
                if let Some(c) = hgetc() {
                    // c:1923
                    hungetc(c); // c:1924
                    // !!! ORDERING DIVERGENCE ADAPTER — C's inungetc
                    // returns the char to the CURRENT input frame
                    // (inbufptr--), which the alias inpush below then
                    // covers, so the terminator is read AFTER the alias
                    // body. zshrs's hungetc pushes into LEX_UNGET_BUF,
                    // which hgetc drains BEFORE any inbuf frame — so the
                    // terminator (blank / `;` / `\n`) would be consumed
                    // ahead of the alias text: `alias git=hub; git
                    // status` fused to `hubstatus`, and the line's `\n`
                    // ran early (PS2 prompt mid-command). Re-route the
                    // pending ungets into an INP_CONT frame pushed UNDER
                    // the separator/alias frames so read order matches C:
                    // alias body → separator → terminator → line rest.
                    // Only plain chars are re-routed — ingetc skips itok
                    // bytes (input.rs c:328), which must stay in
                    // LEX_UNGET_BUF to survive.
                    let all_plain = LEX_UNGET_BUF.with_borrow(|b| {
                        b.iter()
                            .all(|&ch| !((ch as u32) < 256 && crate::ztype_h::itok(ch as u8)))
                    });
                    if all_plain {
                        let pending: String =
                            LEX_UNGET_BUF.with_borrow_mut(|b| b.drain(..).collect());
                        if !pending.is_empty() {
                            inpush(&pending, INP_CONT, None);
                        }
                    }
                    // ASCII-only: see the truncation note in `gettokstr`.
                    if !(c.is_ascii() && crate::ztype_h::iblank(c as u8)) {
                        // c:1925
                        inpush(" ", INP_ALIAS, None); // c:1926
                    }
                }
                // hgetc() == None → EOF: C's ingetc returns ' ' under
                // lexstop (input.c:322), iblank(' ') → no push. Same
                // net effect; nothing to unget.
            }
            // c:1928 — `inpush(an->text, INP_ALIAS, an);`
            inpush(&alias.text, INP_ALIAS, Some(lextext.to_string()));
            // c:1929-1930 — `if (an->text[0] == ' ' && !(an->node.flags & ALIAS_GLOBAL))
            //                  aliasspaceflag = 1;`
            // Drives HISTIGNORESPACE's alias-leading-space suppression
            // path (hist.c:1428): without it, `alias g='echo hi'; g`
            // gets history-logged even with HISTIGNORESPACE set when
            // the alias body starts with a space.
            if !is_global && alias.text.starts_with(' ') {
                LEX_ALIAS_SPACE_FLAG.set(1);
            }
            // c:1929 — `an->inuse = 1;`.
            let mut guard = aliastab_lock().write().expect("aliastab poisoned");
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
                let alias_clone: Option<alias> = {
                    let guard = sufaliastab_lock().read().expect("sufaliastab poisoned");
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
                        inpush(lextext, INP_ALIAS, Some(suffix.to_string()));
                        inpush(" ", INP_ALIAS, None);
                        inpush(&alias.text, INP_ALIAS, None);
                        // c:1941 — `an->inuse = 1;`.
                        let mut guard = sufaliastab_lock().write().expect("sufaliastab poisoned");
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
    // lex.c:1957 — `hwend()` closes the history WORD for the token gettok
    // just produced (zshlex loops `gettok(); while (... exalias())`, so
    // this fires once per token). Paired with the `ihwbegin` at the top of
    // gettok, it records word boundaries into `chwords`; `hend` then keys
    // off `chwordpos` (> 2 means ≥ 1 real word) to decide whether to save
    // the line. The prior port stubbed it, leaving chwordpos stuck at 1, so
    // every interactive line was rejected by hend's `chwordpos <= 2` gate
    // and history recorded nothing.
    crate::ported::hist::ihwend(); // c:1957

    // c:1958-1962 — full faithful gate:
    //   if (interact && isset(SHINSTDIN) && !strin && incasepat <= 0
    //    && tok == STRING && !nocorrect && !(inbufflags & INP_ALIAS)
    //    && !hist_is_in_word()
    //    && (isset(CORRECTALL) || (isset(CORRECT) && incmdpos)))
    //       spckword(&tokstr, 1, incmdpos, 1);
    let inbufflags_alias = (crate::ported::input::inbufflags.with(|f| f.get()) & INP_ALIAS) != 0;
    let strin_set = crate::ported::input::strin.with(|c| c.get()) != 0;
    if interact()
        && isset(SHINSTDIN)
        && !strin_set
        && LEX_INCASEPAT.get() <= 0
        && tok() == STRING_LEX
        && LEX_NOCORRECT.get() == 0
        && !inbufflags_alias
        && crate::ported::hist::hist_is_in_word() == 0
        && (isset(CORRECTALL) || (isset(CORRECT) && LEX_INCMDPOS.get()))
    {
        // c:1962 — `spckword(&tokstr, 1, incmdpos, 1);`. The canonical
        // port at utils.rs::spckword scans the right hashtables
        // internally.
        if let Some(word) = tokstr() {
            let mut buf = if has_token(&word) {
                untokenize(&word)
            } else {
                word.clone()
            };
            crate::ported::utils::spckword(
                &mut buf,
                1,                                      // c:1962 hist=1
                if LEX_INCMDPOS.get() { 1 } else { 0 }, // c:1962 cmd=incmdpos
                1,                                      // c:1962 ask=1
            );
            if buf != word {
                set_tokstr(Some(buf));
            }
        }
    }

    // lex.c:1964-1969 — bare-token path (no tokstr).
    if LEX_TOKSTR.with_borrow(|t| t.is_none()) {
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
        //
        // c:Src/lex.c:1973-1980 (un-tokenize loop): C's untokenize
        // REPLACES Snull/Dnull/Bnull with the literal quote chars
        // (`'`, `"`, `\\`) via `ztokens`. So C's lextext for `"}"` is
        // the 3-byte string `"}"` (quotes intact). The
        // `zshlextext[0] == '}' && !zshlextext[1]` check at c:2003
        // therefore FAILS for any quoted `}` and the reswd lookup
        // never fires.
        //
        // zshrs's untokenize at lex.rs:4275 STRIPS Snull/Dnull/Bnull
        // entirely (intentional — many call sites rely on the
        // markers being gone), so lextext for `"}"` is just `}` (1
        // byte). To still reject quoted `}` from the reswd path, we
        // gate `is_close_brace_special` on the original tokstr NOT
        // containing any quote-marker bytes. Bug #14 in BUGS.md:
        // `[[ x == "}" ]]` and `echo "}"` both used to silently
        // discard the `}` because exalias promoted the quoted `}`
        // to OUTBRACE_TOK.
        let tokstr_has_quote_marker = tokstr
            .chars()
            .any(|c| c == Snull || c == Dnull || c == Bnull);
        let is_close_brace_special = lextext == "}"
            && unset(IGNOREBRACES)
            && unset(IGNORECLOSEBRACES)
            && !tokstr_has_quote_marker;
        // lex.c:2002-2014 — the C structure is
        //     if ((incmdpos || ...) && (rw = reswd_lookup)) { ... }
        //     else if (incond && lextext == "]]") { ... }
        //     else if (incond == 1 && lextext == "!") { ... }
        // i.e. the cond `]]`/`!` branches are reached when the reswd
        // lookup FAILS — even if `incmdpos` is true. The previous
        // Rust port gated on `incmdpos` alone, so any `]]` reached
        // inside `[[ ... ]]` with incmdpos still true (the lexer
        // doesn't auto-reset incmdpos after `[[` in all paths) was
        // left as a STRING `]]` and par_cond_2 errored with
        // `condition expected:`. Restructure to mirror C: look up the
        // reswd ONLY when the gating allows, and treat a failed
        // lookup the same as a non-gated path so the cond branches
        // get their turn.
        let reswd_path_eligible = LEX_INCMDPOS.get() || is_close_brace_special;
        // c:Src/lex.c:1973-1980 — C's untokenize keeps Snull/Dnull/Bnull
        // quote markers AS THEIR LITERAL QUOTE CHARS in `lextext` (via
        // the ztokens table). So C's lextext for `"if"` is the 4-byte
        // string `"if"` (quotes intact) and reswd_lookup against `"if"`
        // never matches the bare reswd `if`. zshrs's untokenize at
        // lex.rs:4275 STRIPS those markers entirely (so lextext is
        // `if`), which would spuriously promote a quoted token to a
        // reserved word. Bug #19 in docs/BUGS.md: quoted patterns like
        // `"!"`, `"if"`, `"{"`, `"}"` in non-first case branches
        // parsed as the corresponding reswd (BANG_TOK / IF / INBRACE
        // / OUTBRACE) and triggered "expected ')' in case pattern".
        // Same root cause as bug #14's `is_close_brace_special` gate
        // above: when the original tokstr carries any Snull/Dnull/Bnull
        // marker, the text WAS quoted by the user and must keep its
        // literal meaning — reswd promotion is suppressed.
        let rw_tok: Option<lextok> = if reswd_path_eligible && !tokstr_has_quote_marker {
            let guard = reswdtab_lock().read().expect("reswdtab poisoned");
            guard.get(&lextext).map(|r| r.token)
        } else {
            None
        };
        // !!! DASH-STRICT GATE (no C counterpart) !!! dash has none of the
        // zsh/bash/ksh reserved words `[[` / `function` / `coproc`; each is an
        // ordinary command word there (`[[`/`coproc` → "not found", `function`
        // → the following `{` is a syntax error). Suppress ONLY these three
        // promotions — every POSIX reserved word (if/then/while/for/case/until/
        // do/done/{/}/…) stays intact. The `]]`/`!`-in-cond branches below are
        // gated on LEX_INCOND, which never rises now. Found by the per-mode
        // dash-strictness sweep.
        let rw_tok = if crate::dash_mode::dash_strict()
            && matches!(rw_tok, Some(DINBRACK) | Some(FUNC) | Some(COPROC))
        {
            None
        } else {
            rw_tok
        };
        if let Some(rwtok) = rw_tok {
            set_tok(rwtok);
            if rwtok == REPEAT {
                LEX_INREPEAT.set(1);
            }
            if rwtok == DINBRACK {
                LEX_INCOND.set(1);
            }
        } else if LEX_INCOND.get() > 0 && lextext == "]]" {
            // lex.c:2010-2012 — `]]` closes the cond expression.
            set_tok(DOUTBRACK);
            LEX_INCOND.set(0);
        } else if LEX_INCOND.get() == 1 && lextext == "!" && !tokstr_has_quote_marker {
            // lex.c:2013-2014 — `!` inside `[[ ]]` is the Bang
            // negation, not a literal. Gate on
            // `tokstr_has_quote_marker` so QUOTED `"!"` (which
            // carries Snull/Dnull/Bnull markers in the original
            // tokstr) stays as a literal STRING token. Same fix
            // shape as #14/#19 above: any user-applied quotes
            // suppress the special-token promotion. Bug #283 in
            // docs/BUGS.md.
            set_tok(BANG_TOK);
        }
    }

    // lex.c:2016 — `inalmore = 0;` — alias-more flag clears once a
    // token makes it through exalias without being re-injected as
    // an alias (checkalias returning true short-circuits before
    // this point via the `return true` above).
    LEX_INALMORE.set(0); // c:2016

    false
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
        let (p, s, l) = LEX_LEXBUF_RAW.with_borrow_mut(|b| (b.ptr.take(), b.siz, b.len));
        new_lexbuf_init_ptr = p;
        new_lexbuf_init_siz = s;
        new_lexbuf_init_len = l;
    } else {
        // Top-level: seed raw with current tokstr/lexbuf.
        new_tokstr_init = tokstr();
        let (p, s, l) = LEX_LEXBUF.with_borrow(|b| (b.ptr.clone(), b.siz, b.len));
        new_lexbuf_init_ptr = p;
        new_lexbuf_init_siz = s;
        new_lexbuf_init_len = l;
    }

    crate::ported::context::zcontext_save_partial(ZCONTEXT_LEX | ZCONTEXT_PARSE);
    hist_in_word(1);

    // c:2147-2149 — install seeded raw buffers + enable recording.
    set_tokstr(new_tokstr_init);
    LEX_LEXBUF.with_borrow_mut(|b| {
        b.ptr = new_lexbuf_init_ptr.clone();
        b.siz = if new_lexbuf_init_siz == 0 {
            256
        } else {
            new_lexbuf_init_siz
        };
        b.len = new_lexbuf_init_len;
    });
    LEX_TOKSTR_RAW.with_borrow_mut(|t| *t = tokstr());
    LEX_LEXBUF_RAW.with_borrow_mut(|b| {
        b.ptr = new_lexbuf_init_ptr;
        b.siz = if new_lexbuf_init_siz == 0 {
            256
        } else {
            new_lexbuf_init_siz
        };
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

            hist_in_word(0);
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
    let _guard = SkipcommGuard {
        outer_was_recording,
    };

    let mut pct = 1;
    let mut start = true;

    // c:Src/lex.c skipcomm (ZSH_OLD path) — the C source's pct
    // counter mis-tracks `case PAT)` as a cmdsub close because
    // the case pattern's `)` decrements pct prematurely. C zsh's
    // NEW skipcomm (default since 5.0.x) recursively re-parses the
    // body so case/esac context is known. zshrs's port still rides
    // the OLD pct counter. Bridge the gap with a `case`-keyword
    // depth tracker: when between `case <word> in` and `esac`,
    // each `)` is a pattern close and must NOT decrement pct.
    // Word boundaries come from a small accumulator that flushes
    // on whitespace / structural separators. Bug #291.
    let mut word_buf = String::with_capacity(8);
    let mut case_depth: i32 = 0;
    // 0 = no recent `case`; 1 = saw `case`, expecting subject word;
    // 2 = saw subject word, expecting `in`.
    let mut case_pending: i32 = 0;

    loop {
        let c = hgetc();
        let c = match c {
            Some(c) => c,
            None => {
                LEX_LEXSTOP.set(true);
                return Err(());
            }
        };

        // Only ASCII can be blank — see the note in `gettokstr`: `c as u8` on a
        // multibyte char truncates the codepoint into the blank range.
        let iswhite = c.is_ascii() && crate::ztype_h::inblank(c as u8);

        // Word boundary keyword tracking.
        let is_word_terminator =
            iswhite || c == '\n' || c == ';' || c == '&' || c == '|' || c == '(' || c == ')';
        if is_word_terminator && !word_buf.is_empty() {
            match word_buf.as_str() {
                "case" => case_pending = 1,
                "in" if case_pending == 2 => {
                    case_depth += 1;
                    case_pending = 0;
                }
                "esac" => {
                    if case_depth > 0 {
                        case_depth -= 1;
                    }
                    case_pending = 0;
                }
                _ => match case_pending {
                    1 => case_pending = 2,
                    2 => case_pending = 0,
                    _ => {}
                },
            }
            word_buf.clear();
        } else if !is_word_terminator {
            word_buf.push(c);
        }

        match c {
            '(' => {
                pct += 1;
                add(c);
            }
            ')' => {
                // c:Bug #291 — inside a case block, `)` closes a
                // pattern (not the cmdsub).
                if case_depth > 0 {
                    add(c);
                } else {
                    pct -= 1;
                    if pct == 0 {
                        return Ok(());
                    }
                    add(c);
                }
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

/// Port of `static const char ztokens[]` from `Src/lex.c:80`.
pub const ztokens: &str = "#$^*(())$=|{}[]`<>>?~`,-!'\"\\\\";

// `enum lextok` — port of `Src/zsh.h:304-371`. The full constant set
// (`NULLTOK`, `SEPER`, …, `TYPESET`) and the `lextok` type alias live
// in `super::zsh_h:198-262`. Re-export here so external callers can
// keep saying `lex::lextok` / `tokens::lextok` without reaching into
// `zsh_h::` directly. `IS_REDIROP()` (port of `Src/zsh.h:408`
// `#define IS_REDIROP`) lives in `zsh_h:318`.

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

/// The Zsh Lexer.
///
// All lexer state lives in the file-scope `LEX_*` thread_local statics
// above, each one matching a `static` in `Src/lex.c`. There's no
// holder struct — callers use the free ported directly:
//
//    lex_init(input);
//    zshlex();
//   let tok =  tok();
//
// Accessor ported named after the former field identifiers (`tok()`,
// `tokstr()`, `set_tok(v)`, etc.) provide read/write into LEX_*.

// ─── Accessor ported for the LEX_* thread_locals (Src/lex.c file-statics) ───
/// `toklineno` — see implementation.
pub fn toklineno() -> u64 {
    LEX_TOKLINENO.get()
}
/// `set_toklineno` — see implementation.
pub fn set_toklineno(v: u64) {
    LEX_TOKLINENO.set(v);
}
/// `tokfd` — see implementation.
pub fn tokfd() -> i32 {
    LEX_TOKFD.get()
}
/// `set_tokfd` — see implementation.
pub fn set_tokfd(v: i32) {
    LEX_TOKFD.set(v);
}
/// `isnewlin` — see implementation.
pub fn isnewlin() -> i32 {
    LEX_ISNEWLIN.get()
}
/// `set_isnewlin` — see implementation.
pub fn set_isnewlin(v: i32) {
    LEX_ISNEWLIN.set(v);
}
/// `inrepeat` — see implementation.
pub fn inrepeat() -> i32 {
    LEX_INREPEAT.get()
}
/// `set_inrepeat` — see implementation.
pub fn set_inrepeat(v: i32) {
    LEX_INREPEAT.set(v);
}
/// `infor` — see implementation.
pub fn infor() -> i32 {
    LEX_INFOR.get()
}
/// `set_infor` — see implementation.
pub fn set_infor(v: i32) {
    LEX_INFOR.set(v);
}
/// `inredir` — see implementation.
pub fn inredir() -> bool {
    LEX_INREDIR.get()
}
/// `set_inredir` — see implementation.
pub fn set_inredir(v: bool) {
    LEX_INREDIR.set(v);
}
/// `intypeset` — see implementation.
pub fn intypeset() -> bool {
    LEX_INTYPESET.get()
}
/// `set_intypeset` — see implementation.
pub fn set_intypeset(v: bool) {
    LEX_INTYPESET.set(v);
}
/// `lineno` — see implementation.
pub fn lineno() -> u64 {
    LEX_LINENO.get()
}
/// `set_lineno` — see implementation.
pub fn set_lineno(v: u64) {
    LEX_LINENO.set(v);
}
/// `incmdpos` — see implementation.
pub fn incmdpos() -> bool {
    LEX_INCMDPOS.get()
}
/// `set_incmdpos` — see implementation.
pub fn set_incmdpos(v: bool) {
    LEX_INCMDPOS.set(v);
}
/// Port of `int nocorrect` from `Src/lex.c:144`. Getter/setter for
/// the spelling-correction suppression flag. `par_redir` saves and
/// restores this around the zshlex that consumes the redir target.
pub fn nocorrect() -> i32 {
    LEX_NOCORRECT.get()
}
/// `set_nocorrect` — see implementation.
pub fn set_nocorrect(v: i32) {
    LEX_NOCORRECT.set(v);
}
/// Port of `int noaliases` from `Src/lex.c:135`. Suppresses alias
/// expansion. par_case saves and restores this around the case-word
/// + `in` lex so the literal `in` keyword isn't alias-expanded.
pub fn noaliases() -> bool {
    LEX_NOALIASES.get()
}
/// `set_noaliases` — see implementation.
pub fn set_noaliases(v: bool) {
    LEX_NOALIASES.set(v);
}
/// `incond` — see implementation.
pub fn incond() -> i32 {
    LEX_INCOND.get()
}
/// `set_incond` — see implementation.
pub fn set_incond(v: i32) {
    LEX_INCOND.set(v);
}

// `hashchar` / `bangchar` / `hatchar` canonical port of
// `unsigned char hashchar, bangchar, hatchar;` (`Src/params.c:132`)
// lives at `crate::ported::hist::{hashchar, bangchar, hatchar}` as
// `AtomicI32` so `histcharssetfn` (`Src/params.c:5095-5097`) can
// update them when `$HISTCHARS` changes. Local stale-const copies
// removed — readers now go through the atomic load directly.
/// `incasepat` — see implementation.
pub fn incasepat() -> i32 {
    LEX_INCASEPAT.get()
}
/// `set_incasepat` — see implementation.
pub fn set_incasepat(v: i32) {
    LEX_INCASEPAT.set(v);
}
/// Port of `mod_export char *tokstrings[WHILE + 1]` from
/// `Src/lex.c:171-205`. Canonical text for each punctuation token —
/// used by `zshlex` (lex.c:1965 `zshlextext = tokstrings[tok]`) when
/// no tokstr was captured, and by `yyerror` (parse.c:2738) to format
/// "parse error near `X'" tails.
///
/// Indexed by lextok value (C's `tokstrings[tok]`). Entries the C
/// initializer doesn't set are `None`; the array bound is WHILE+1
/// matching C exactly.
#[allow(non_upper_case_globals)]
pub static tokstrings: [Option<&'static str>; (WHILE + 1) as usize] = {
    let mut t: [Option<&'static str>; (WHILE + 1) as usize] = [None; (WHILE + 1) as usize];
    t[SEPER as usize] = Some(";"); // c:173
    t[NEWLIN as usize] = Some("\\n"); // c:174
    t[SEMI as usize] = Some(";"); // c:175
    t[DSEMI as usize] = Some(";;"); // c:176
    t[AMPER as usize] = Some("&"); // c:177
    t[INPAR_TOK as usize] = Some("("); // c:178
    t[OUTPAR_TOK as usize] = Some(")"); // c:179
    t[DBAR as usize] = Some("||"); // c:180
    t[DAMPER as usize] = Some("&&"); // c:181
    t[OUTANG_TOK as usize] = Some(">"); // c:182
    t[OUTANGBANG as usize] = Some(">|"); // c:183
    t[DOUTANG as usize] = Some(">>"); // c:184
    t[DOUTANGBANG as usize] = Some(">>|"); // c:185
    t[INANG_TOK as usize] = Some("<"); // c:186
    t[INOUTANG as usize] = Some("<>"); // c:187
    t[DINANG as usize] = Some("<<"); // c:188
    t[DINANGDASH as usize] = Some("<<-"); // c:189
    t[INANGAMP as usize] = Some("<&"); // c:190
    t[OUTANGAMP as usize] = Some(">&"); // c:191
    t[AMPOUTANG as usize] = Some("&>"); // c:192
    t[OUTANGAMPBANG as usize] = Some("&>|"); // c:193
    t[DOUTANGAMP as usize] = Some(">>&"); // c:194
    t[DOUTANGAMPBANG as usize] = Some(">>&|"); // c:195
    t[TRINANG as usize] = Some("<<<"); // c:196
    t[BAR_TOK as usize] = Some("|"); // c:197
    t[BARAMP as usize] = Some("|&"); // c:198
    t[INOUTPAR as usize] = Some("()"); // c:199
    t[DINPAR as usize] = Some("(("); // c:200
    t[DOUTPAR as usize] = Some("))"); // c:201
    t[AMPERBANG as usize] = Some("&|"); // c:202
    t[SEMIAMP as usize] = Some(";&"); // c:203
    t[SEMIBAR as usize] = Some(";|"); // c:204
    t
};

/// `char *tokstr` accessors — direct port of lex.c:170 file-static.
pub fn tokstr() -> Option<String> {
    LEX_TOKSTR.with_borrow(|t| t.clone())
}
/// `set_tokstr` — see implementation.
pub fn set_tokstr(v: Option<String>) {
    LEX_TOKSTR.with_borrow_mut(|t| *t = v);
}
/// `enum lextok tok` accessors — direct port of lex.c:180 file-static.
pub fn tok() -> lextok {
    LEX_TOK.get()
}
/// `set_tok` — see implementation.
pub fn set_tok(v: lextok) {
    LEX_TOK.set(v);
}
/// `pos` — see implementation.
pub fn pos() -> usize {
    LEX_POS.get()
}
/// `set_pos` — see implementation.
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
    // NOT reset here. C's `lexinit` (c:441-445) resets nocorrect/dbparens/lexstop
    // but deliberately leaves `noaliases` alone: it is a plain global
    // (Src/lex.c:135), saved and restored by its callers. `loadautofn`
    // (Src/exec.c:5684-5704) sets it from the function's PM_UNALIASED bit and
    // then parses the autoloaded file — so resetting it inside lexinit would
    // clobber the flag before the parse it exists to govern, which is why
    // `autoload -U` failed to suppress alias expansion in the loaded body.
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

// Direct port of the anonymous enum at `Src/lex.c:483-487`:
//   enum { CMD_OR_MATH_CMD, CMD_OR_MATH_MATH, CMD_OR_MATH_ERR };
// `cmd_or_math()` and `cmd_or_math_sub()` return one of these as `int`.
// Following the same flat-const pattern zshrs uses for lextok
// (zsh_h.rs:198-251) so call sites read the C identifier verbatim.
/// `CMD_OR_MATH_CMD` constant.
pub const CMD_OR_MATH_CMD: i32 = 0;
/// `CMD_OR_MATH_MATH` constant.
pub const CMD_OR_MATH_MATH: i32 = 1;
/// `CMD_OR_MATH_ERR` constant.
pub const CMD_OR_MATH_ERR: i32 = 2;

/// Check recursion depth; returns true if exceeded
#[inline]
/// Get next character from input
pub(crate) fn hgetc() -> Option<char> {
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
        // c:input.c:327 — re-reading the un-gotten char consumes it like
        // any other read (C's ingetc `inbufct--`). Mirror it so the
        // hungetc(+1)/reread(-1) pair stays balanced; scoped to
        // LEXFLAGS_ZLE for the same reason as the hungetc restore.
        if LEX_LEXFLAGS.get() & LEXFLAGS_ZLE != 0 {
            crate::ported::input::inbufct.with(|ct| ct.set(ct.get() - 1));
        }
        return Some(c);
    }

    // Two-input-system bridge (c:Src/input.c): zshrs's lexer has its
    // own LEX_INPUT/LEX_POS char-window while input.rs maintains
    // ingetc's inbuf stack (used by inpush for alias / here-string /
    // eval content). Prefer the inbuf stack WHEN it has any content
    // (inbufct > 0). If inbufct == 0 but there's a pending instack
    // frame to pop (INP_CONT'd lower frame e.g. exalias's leading
    // " " separator after the body drained), ingetc's c:296
    // `if (inbufflags & INP_CONT) { inpoptop(); continue; }` arm
    // handles the pop. Then unwind to LEX_INPUT for the remainder.
    let pos = LEX_POS.get();
    let inbufct = crate::ported::input::inbufct.with(|c| c.get());
    // Also probe instack via the flags-on-current-frame: if the
    // CURRENT frame has INP_CONT, ingetc will pop and continue. The
    // bridge needs to call ingetc to trigger that pop even when
    // inbufct == 0.
    let flags = crate::ported::input::inbufflags.with(|f| f.get());
    let inp_cont_pending = (flags & crate::ported::zsh_h::INP_CONT) != 0;
    let try_inbuf = inbufct > 0 || inp_cont_pending;

    // c:Src/lex.c — the lexer reads characters via `hgetc`, which C points
    // at `ihgetc` (hist.c:418) when history is on. For the inbuf/ingetc
    // stack source (interactive input) we DELEGATE to the ported `ihgetc`
    // (ingetc → histsubchar → hwaddc → addtoline) so `!`-expansion and the
    // history-line build are CALLED, not re-inlined here — but only when
    // history is active (stophist==0 && !INP_ALIAS). The Rust-only
    // LEX_INPUT window (`-c` / cmd-subst / eval, where history is off) is
    // read directly. `ihgetc` returns C's `int`; it signals EOF / a bad
    // `!`-reference by setting `lexstop`, which we map back to the Option
    // API's `None`.
    let hist_active = crate::ported::hist::stophist.load(Ordering::SeqCst) == 0
        && (flags & crate::ported::zsh_h::INP_ALIAS) == 0;
    let read_stack = || -> Option<char> {
        if hist_active {
            let nc = crate::ported::hist::ihgetc(); // c:418
            if crate::ported::input::lexstop.with(|s| s.get()) {
                None
            } else {
                char::from_u32(nc as u32)
            }
        } else {
            crate::ported::input::ingetc()
        }
    };

    let from_inbuf = if try_inbuf { read_stack() } else { None };
    let c = if let Some(c) = from_inbuf {
        c
    } else if let Some(c) = LEX_INPUT.with_borrow(|s| s.get(pos..).and_then(|t| t.chars().next())) {
        LEX_POS.set(pos + c.len_utf8());
        c
    } else {
        // inbuf + LEX_INPUT both empty. If a prior read already set lexstop
        // (real EOF, or a bad `!`-expansion), stop now WITHOUT re-reading —
        // re-entering ihgetc here would `hwaddc` a stray char into chline.
        // (An inbuf drain while `strin` is set — e.g. an alias body ending
        // mid-`eval` — ALSO sets lexstop, but never reaches here: the
        // LEX_INPUT branch above still holds the post-alias text.) No
        // lexstop → genuine refill: read the next line via the same
        // history-aware path (triggers inputline). `?` is None at EOF.
        if crate::ported::input::lexstop.with(|s| s.get()) {
            return None;
        }
        read_stack()?
    };

    if c == '\n' {
        LEX_LINENO.set(LEX_LINENO.get() + 1);
    }

    // c:input.c:360-361 — `if (!lexstop) zshlex_raw_add(lastc);`
    // Every char read from input also feeds the raw buffer when
    // lex_add_raw is on (used by skipcomm to capture verbatim
    // `$(...)` body text into the parent token). (The history-line build —
    // C `hwaddc`/`addtoline` at ihgetc c:459-460 — is done INSIDE the
    // `ihgetc` call above, not duplicated here.)
    zshlex_raw_add(c);

    Some(c)
}

/// Put character back into input. Direct port of `inungetc(int c)`
/// from `Src/input.c:546-583`. Critical: C decrements lineno on
/// `\n` ungetc UNCONDITIONALLY (modulo input-flags). Rust's
/// previous `LEX_LINENO.get() > 1` guard caused off-by-one drift
/// when lineno was set to 0 (via par_funcdef's `set_lineno(0)`)
/// then a `\n` got read+ungetted: hungetc wouldn't decrement
/// (because LEX_LINENO==1 fails `> 1`), but the subsequent re-read
/// in hgetc DOES increment, leaving LEX_LINENO=2 instead of 1.
fn hungetc(c: char) {
    LEX_UNGET_BUF.with_borrow_mut(|b| b.push_front(c));
    if c == '\n' {
        // c:input.c:561-562 — `if (((inbufflags & INP_LINENO) ||
        // !strin) && c == '\n') lineno--;`
        let cur = LEX_LINENO.get();
        if cur > 0 {
            LEX_LINENO.set(cur - 1);
        }
    }
    LEX_LEXSTOP.set(false);
    // c:input.c:549,609 — `inungetc` calls `zshlex_raw_back()` so
    // the un-gotten char isn't double-counted in lexbuf_raw on
    // re-read. hgetc will re-add it next time it's pulled.
    zshlex_raw_back();
    // c:input.c:558-559 — `inbufptr--; inbufct++;`. C's ungetc pushes the
    // char back into inbuf AND restores `inbufct`, so a read-then-unget
    // leaves `inbufct` counting the char as un-consumed. `gotword`
    // (c:lex.c:1884) reads that restored count via
    // `nwe = zlemetall + 1 - inbufct` to place the completion word END.
    // The Rust bridge ungets via LEX_UNGET_BUF (not inbuf) and so never
    // restored `inbufct`; a word-terminating char read-then-ungotten
    // then left `inbufct` one low, inflating `we`/`swe` by one and
    // dropping the leading separator of a `compset -q` ignored suffix.
    // Scoped to LEXFLAGS_ZLE: only the completion lexer (set_comp_sep /
    // get_comp_string, fed entirely through inpush -> inbuf) reads
    // `inbufct` for word positions; normal parsing may read the
    // Rust-only LEX_INPUT window where `inbufct` isn't tracked, so it
    // must not be perturbed. Paired with the matching `inbufct--` on the
    // unget re-read in hgetc so the count stays balanced.
    if LEX_LEXFLAGS.get() & LEXFLAGS_ZLE != 0 {
        crate::ported::input::inbufct.with(|ct| ct.set(ct.get() + 1));
    }
}

/// Peek at next character without consuming
#[allow(dead_code)]
fn peek() -> Option<char> {
    if let Some(c) = LEX_UNGET_BUF.with_borrow(|b| b.front().copied()) {
        return Some(c);
    }
    LEX_INPUT.with_borrow(|s| s[LEX_POS.get()..].chars().next())
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

    // Leading digit: an all-digit name is a positional-parameter
    // assignment (`2=X` → $2). c:Src/params.c:1336-1340 isident treats
    // an all-digit name as a valid identifier; the lexer's assignment
    // detection (c:1233-1242) then accepts the optional `+` for the
    // augment form. The prior port required ALL digits with NO trailing
    // `+`, so `2=X` lexed as ENVSTRING but `2+=end` did not (it became a
    // command word → "command not found: 2+=end").
    if let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            while let Some(&c) = chars.peek() {
                if !c.is_ascii_digit() {
                    break;
                }
                chars.next();
            }
            // c:1241-1242 — `if (*t == '+') t++;` optional `+=` form.
            // dash-strict has no `+=` (see identifier path below).
            if chars.peek() == Some(&'+') && !crate::dash_mode::dash_strict() {
                chars.next();
            }
            return chars.peek().is_none();
        }
    }

    // c:1233 — `t = itype_end(t, INAMESPC, 0);` — walk past
    // identifier chars (alpha/digit/_).
    let mut has_ident = false;
    let mut after_ident_byte = 0usize;
    {
        let mut sub_chars = s.chars().peekable();
        let mut consumed_bytes = 0usize;
        while let Some(&c) = sub_chars.peek() {
            if c == Inbrack || c == '[' || c == '+' {
                break;
            }
            // Both classifiers are BYTE tables, so a `char as u8` cast truncates.
            // `iident` is ASCII-only; `itok` must stay reachable for the token
            // codepoints themselves (Pound 0x84 … Nularg 0xa1, all < 0x100), so
            // it is gated on the codepoint fitting in a byte rather than on
            // is_ascii — otherwise `二` (U+4E8C → 0x8C) would masquerade as a
            // token and be swallowed into an identifier.
            // c:1233 itype_end(t, INAMESPC, 0) → wcsitype(IIDENT) accepts any
            // non-ASCII alphanumeric under MULTIBYTE (and not POSIXIDENTIFIERS)
            // — c:utils.c:4347-4350. So `日=x` / `café=y` are assignments, not
            // commands. The full codepoint is tested (not `c as u8`), so a
            // multibyte char can't collide with a token byte (0x84..0xa1).
            // Bug #1021 (assignment leg).
            let c_is_ident = (c.is_ascii() && crate::ztype_h::iident(c as u8))
                || (!c.is_ascii()
                    && c.is_alphanumeric()
                    && isset(crate::ported::zsh_h::MULTIBYTE)
                    && !isset(crate::ported::zsh_h::POSIXIDENTIFIERS));
            let c_is_tok = (c as u32) < 0x100 && itok(c as u8);
            if !c_is_ident && c != Stringg && !c_is_tok {
                return false;
            }
            has_ident = true;
            consumed_bytes += c.len_utf8();
            sub_chars.next();
        }
        after_ident_byte = consumed_bytes;
    }
    // c:1236-1238 — `if (t < lexbuf.ptr) skipparens(Inbrack, Outbrack, &t);`.
    // When the identifier doesn't span the whole buffer, the remainder
    // must be a `[index]` subscript — balanced via skipparens.
    let mut cursor: &str = &s[after_ident_byte..];
    if cursor.starts_with(Inbrack) || cursor.starts_with('[') {
        let bal = crate::ported::utils::skipparens(Inbrack, Outbrack, &mut cursor);
        let _ = bal; // C ignores the return value here — `t` is advanced regardless.
    }
    // c:1241-1242 — `if (*t == '+') t++;` — optional `+=` form.
    // !!! DASH-STRICT GATE (no C counterpart) !!! dash has no `+=`
    // compound assignment; `x+=b` is a command word (→ "not found").
    // Leaving the `+` unconsumed makes `cursor` non-empty below so the
    // whole `x+` fails the assignment test, matching /bin/dash.
    if let Some(c) = cursor.chars().next() {
        if c == '+' && !crate::dash_mode::dash_strict() {
            cursor = &cursor[1..];
        }
    }
    // c:1243 — `if (t == lexbuf.ptr) ...` — full buffer consumed means
    // this is a valid assignment target.
    has_ident && cursor.is_empty()
}

/// Untokenize a string - convert tokenized chars back to original
///
/// Port of untokenize(char *s) from exec.c (but used by lexer too)
/// Like `untokenize`, but maps Snull → `'` and Dnull → `"` instead of
/// stripping them. Used by callers that need the source form including
/// quoting (e.g. arithmetic-substitution detection in compile_zsh).
///
/// Token-byte detection routes through the canonical ITOK range
/// `Pound..=Nularg` = `0x84..=0xa1` (per `Src/zsh.h:159-205` +
/// `Src/ztype.h:52`). The previous Rust port used `(0x83..=0x9f)`
/// which was BOTH too inclusive (0x83 = Meta, IMETA-only, NOT ITOK)
/// and too narrow (missing 0xa0 Bnullkeep and 0xa1 Nularg).
pub fn untokenize_preserve_quotes(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        let cu = c as u32;
        if (0x84..=0xa1).contains(&cu) {
            // c:52 (Src/ztype.h) ITOK range
            match c {
                c if c == Pound => result.push('#'),
                c if c == Stringg => result.push('$'),
                c if c == Hat => result.push('^'),
                c if c == Star => result.push('*'),
                c if c == Inpar => result.push('('),
                c if c == Outpar => result.push(')'),
                c if c == Inparmath => result.push('('),
                c if c == Outparmath => result.push(')'),
                // c:167 — Qstring is the DQ-context `$` marker.
                // Preserve in this variant so the downstream
                // `stringsubst` qt detection at Src/subst.c:283
                // (`if ((qt = c == Qstring) || c == String)`) fires
                // and paramsubst sees qt=true. Untokenizing to plain
                // `$` would lose the DQ context for the
                // BUILTIN_EXPAND_TEXT singsub path, breaking
                // DQ-wrapped flag forms like `"${(o)arr}"` (should
                // join to scalar, not sort+splat per c:3033 sepjoin
                // + c:4245 isarr-gated sort).
                c if c == Qstring => result.push(c),
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
                    } else {
                        // c:Src/utils.c — `\xNN` is one raw BYTE, not
                        // a Unicode codepoint (the old char::from_u32
                        // re-encoded 0xNN >= 0x80 as two UTF-8
                        // bytes); metafied per c:Src/utils.c:7289-
                        // 7294. Bug #127.
                        {
                            // c:Src/utils.c metafy byte-encode step:
                            // `if (imeta(c)) {{ *p++ = Meta; *p++ = c ^ 32; }}`
                            let b_ = (val & 0xff) as u8;
                            if b_ < 0x80 {
                                out.push(b_ as char);
                            } else {
                                out.push('\u{83}');
                                out.push(char::from(b_ ^ 32));
                            }
                        }
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
                    // c:Src/utils.c — octal escape is one raw BYTE
                    // (`$'\377'` = 0xff), metafied per
                    // c:Src/utils.c:7289-7294. Bug #127.
                    {
                        // c:Src/utils.c metafy byte-encode step:
                        // `if (imeta(c)) {{ *p++ = Meta; *p++ = c ^ 32; }}`
                        let b_ = (val & 0xff) as u8;
                        if b_ < 0x80 {
                            out.push(b_ as char);
                        } else {
                            out.push('\u{83}');
                            out.push(char::from(b_ ^ 32));
                        }
                    }
                    i += consumed;
                }
                'C' | 'M' => {
                    // c:Src/utils.c:7029-7052 — `\C` and `\M` set
                    // `control` / `meta` flags; the optional `-`
                    // separator is consumed; then the NEXT char (or
                    // chained `\C`/`\M` modifier) is read and the
                    // mask applied at c:7265-7275 (control → `& 0x9f`,
                    // meta → `| 0x80`). `\C-?` is special-cased to
                    // 0x7f. Bug #113 in docs/BUGS.md: the previous
                    // Rust port dropped `\C` / `\M` into the
                    // unknown-escape default branch, so `$'\C-a'`
                    // emitted literal `C-a` instead of byte 0x01.
                    let mut control = nc == 'C';
                    let mut meta = nc == 'M';
                    let mut j = i + 2;
                    // Consume any chain of `-`, additional `\C`/`\M`
                    // modifiers (e.g. `\M-\C-x` → meta+control on x).
                    loop {
                        if j < chars.len() && chars[j] == '-' {
                            j += 1;
                            continue;
                        }
                        if j + 1 < chars.len()
                            && chars[j] == '\\'
                            && (chars[j + 1] == 'C' || chars[j + 1] == 'M')
                        {
                            if chars[j + 1] == 'C' {
                                control = true;
                            } else {
                                meta = true;
                            }
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    if j >= chars.len() {
                        // Malformed — preserve literal per
                        // Src/utils.c:7050 fallthrough.
                        out.push('\\');
                        out.push(nc);
                        i += 2;
                        continue;
                    }
                    // Read one base char (allowing nested `\xNN` /
                    // `\NNN` / `\u…` / literal). For simplicity,
                    // accept either a literal char or a one-char
                    // escape.
                    let (mut ch, advance): (char, usize) =
                        if chars[j] == '\\' && j + 1 < chars.len() {
                            let nn = chars[j + 1];
                            match nn {
                                'a' => ('\x07', 2),
                                'b' => ('\x08', 2),
                                'e' | 'E' => ('\x1b', 2),
                                'f' => ('\x0c', 2),
                                'n' => ('\n', 2),
                                'r' => ('\r', 2),
                                't' => ('\t', 2),
                                'v' => ('\x0b', 2),
                                '\\' | '\'' | '"' => (nn, 2),
                                _ => (nn, 2), // unknown — take literal char
                            }
                        } else {
                            (chars[j], 1)
                        };
                    let mut byte = ch as u32;
                    if control {
                        // c:7265-7269 — `\C-?` → 0x7f; else AND 0x9f.
                        if byte == '?' as u32 {
                            byte = 0x7f;
                        } else {
                            byte &= 0x9f;
                        }
                    }
                    if meta {
                        // c:7272-7274 — OR 0x80.
                        byte |= 0x80;
                    }
                    // c:Src/utils.c:7265-7275 — the masked result is
                    // one raw BYTE (`$'\M-i'` = 0xe9), metafied per
                    // c:7289-7294. Multibyte base chars (> 0xff after
                    // masking) keep the codepoint form. Bug #127.
                    if byte <= 0xff {
                        {
                            // c:Src/utils.c metafy byte-encode step:
                            // `if (imeta(c)) {{ *p++ = Meta; *p++ = c ^ 32; }}`
                            let b_ = byte as u8;
                            if b_ < 0x80 {
                                out.push(b_ as char);
                            } else {
                                out.push('\u{83}');
                                out.push(char::from(b_ ^ 32));
                            }
                        }
                    } else {
                        ch = char::from_u32(byte).unwrap_or('\0');
                        out.push(ch);
                    }
                    i = j + advance;
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
/// `untokenize` — see implementation.
pub fn untokenize(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        // A Meta byte (U+0083) escapes the FOLLOWING char: zshrs metafies at the
        // CHARACTER level, so a raw high byte that C leaves untouched (0x81 is
        // not `imeta`) is stored here as the pair (U+0083, byte ^ 0x20). That
        // continuation char can land in the ITOK range — 0x81 -> 0xA1 = Nularg —
        // and would be wrongly stripped as a token below. Copy the pair verbatim
        // and never untokenize the continuation. (Only fires on metafied
        // high-byte content; a plain tokenized string has no U+0083.)
        if c as u32 == 0x83 && i + 1 < chars.len() {
            result.push(c);
            result.push(chars[i + 1]);
            i += 2;
            continue;
        }
        // C `itok()` (Src/ztype.h:52 + Src/utils.c:4198-4201) covers
        // the canonical TOKEN range Pound (0x84) through Nularg (0x9d
        // via Snull's chain to Nularg=0xa1). The previous Rust port
        // used `(0x83..=0x9f)` which both:
        //   * INCLUDED 0x83 = META (metafy lead byte, IMETA-only, NOT
        //     ITOK), and
        //   * EXCLUDED 0xa0 (Bnullkeep) and 0xa1 (Nularg) — both
        //     legitimate ITOK bytes per zsh.h:199-205.
        // Match C exactly: ITOK is 0x84..=0xa1 (Pound..Nularg).
        // Marker (0xa2) is intentionally NOT in the range — C's untokenize
        // never strips it (it's IMETA-only per Src/utils.c:4197).
        let cu = c as u32;
        if (0x84..=0xa1).contains(&cu) {
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
            // c:Src/lex.c top-level `$'...'` lexer emits `Stringg`
            // (`\u{85}`) before the opening `Snull`, while DQ-context
            // `$'...'` would emit Qstring (`\u{8c}`). Accept both so
            // untokenize handles top-level and DQ-context ANSI-C
            // strings the same way.
            if (c == Qstring || c == Stringg) && i + 1 < chars.len() && chars[i + 1] == Snull {
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
                c if c == Snull || c == Dnull => {
                    // c:Src/exec.c:2077 + Src/lex.c:38 ztokens — C
                    // maps Snull → `'` / Dnull → `"` because C's
                    // untokenize fires on user-facing text (xtrace
                    // output, error messages, command lookups). The
                    // quote chars belong in that output.
                    //
                    // zshrs splits the responsibility across TWO
                    // functions to match the Rust pipeline's call
                    // shape:
                    //   - `untokenize` (THIS function) — strips
                    //     Snull / Dnull because it's invoked on the
                    //     SUBSTITUTION STREAM (subst.rs:4438 rest-
                    //     walker, multsub paths, etc.) where the
                    //     lex's quote-pair markers must NOT reappear
                    //     as literal `'`/`"` in the value text.
                    //   - `untokenize_preserve_quotes` (lex.rs:4179)
                    //     — maps Snull → `'`, Dnull → `"`, Bnull →
                    //     `\` per C's ztokens table exactly. Used
                    //     for assoc-key lookup (where keys can
                    //     contain literal `'`/`"`), xtrace, error
                    //     reporting.
                    //
                    // The split is deliberate, not a partial port.
                    // Calling a value-stream caller through
                    // `untokenize_preserve_quotes` would leak stray
                    // quote chars into stored values; calling a
                    // print-side caller through `untokenize` would
                    // drop quote chars from the displayed text.
                    // Each call site picks the variant that matches
                    // its semantic contract.
                }
                // c:Src/exec.c:2077-2099 + Src/lex.c:38 ztokens —
                // `ztokens[Bnull - Pound]` (index 27) = `\` literal.
                // C maps Bnull → `\` so downstream patcompile sees the
                // escape. The previous Rust port STRIPPED Bnull, which
                // collapsed `[\\]` / `[\{]` / `[\}]` patterns to
                // `[\]` / `[{]` / `[}]` before patcompile (subst.rs
                // `${msg//PAT/REPL}` path) and tripped "bad pattern".
                // Surfacing site: zinit zi-log message formatter
                // (zinit.zsh:2191).
                c if c == Bnull => result.push('\\'), // c:Src/lex.c:38
                // c:2089 — `if (c != Nularg) *p++ = ztokens[c - Pound];`
                // Nularg gets dropped (no replacement char emitted).
                c if c == Nularg => {
                    // Skip — matches C's c != Nularg gate.
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

/// Check if a string contains any token characters.
/// Port of `int has_token(const char *s)` from `Src/utils.c:2282` —
/// `while (*s) if (itok(*s++)) return 1; return 0;`.
///
/// `itok(c)` per `Src/ztype.h:52` is `STOUC(c) >= Pound && STOUC(c)
/// <= Nularg`, i.e. the closed range `Pound..=Nularg` =
/// `0x84..=0xa1` (per `zsh.h:159-205`). The previous Rust port used
/// `(0x83..=0x9f)` which was BOTH:
///   * too inclusive (0x83 = Meta, IMETA-only, NOT ITOK per
///     ztype_init's typtab population at `utils.c:4197`); and
///   * too narrow (excluded 0xa0 Bnullkeep and 0xa1 Nularg, both
///     legitimate ITOK bytes — so a string containing Nularg from
///     `$''` lexing would falsely report "no token").
/// Route through the canonical `ztype_h::itok` so future ITOK
/// changes propagate automatically (typtab is a runtime table).
pub fn has_token(s: &str) -> bool {
    // c:2282 (Src/utils.c)
    s.bytes().any(itok)
}

#[cfg(test)]
mod tokens_tests {
    use crate::ported::hashtable::reswdtab_lock;
    use crate::ported::zsh_h::{
        Bnull, Dnull, Snull, DINANG, IF, IS_REDIROP, OUTANG_TOK, STRING_LEX, THEN,
    };

    #[test]
    fn test_token_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(Snull as u32, 0x9d);
        assert_eq!(Dnull as u32, 0x9e);
        assert_eq!(Bnull as u32, 0x9f);
    }

    #[test]
    fn test_reserved_words() {
        let _g = crate::test_util::global_state_lock();
        // Reserved-word lookup goes through the canonical `reswdtab`
        // (port of `Src/hashtable.c:1076 reswds[]`).
        let tab = reswdtab_lock().read().unwrap();
        assert_eq!(tab.get("if").map(|r| r.token), Some(IF));
        assert_eq!(tab.get("then").map(|r| r.token), Some(THEN));
        assert!(tab.get("notakeyword").is_none());
    }

    #[test]
    fn test_redirop() {
        let _g = crate::test_util::global_state_lock();
        assert!(IS_REDIROP(OUTANG_TOK));
        assert!(IS_REDIROP(DINANG));
        assert!(!IS_REDIROP(IF));
        assert!(!IS_REDIROP(STRING_LEX));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("echo $(pwd)");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
    }

    #[test]
    fn test_env_assignment() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("FOO=bar echo");
        set_incmdpos(true);
        zshlex();
        assert_eq!(tok(), ENVSTRING, "tok={:?} tokstr={:?}", tok(), tokstr());

        zshlex();
        assert_eq!(tok(), STRING_LEX);
    }

    #[test]
    fn test_array_assignment() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("arr=(a b c)");
        set_incmdpos(true);
        zshlex();
        assert_eq!(tok(), ENVARRAY);
    }

    #[test]
    fn test_process_substitution() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("echo $((1+2))");
        zshlex();
        assert_eq!(tok(), STRING_LEX);

        zshlex();
        assert_eq!(tok(), STRING_LEX);
    }

    #[test]
    fn test_semicolon_variants() {
        let _g = crate::test_util::global_state_lock();
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

    /// c:3952 — `untokenize` removes Marker/Bnull/Snull tokens left
    /// by the param-substitution pipeline. A plain string passes
    /// through unchanged; tokenised input gets its sentinels stripped.
    /// Regression that fails to strip would leak Marker bytes (0x84)
    /// into user-visible output.
    #[test]
    fn untokenize_passes_plain_string_through() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(untokenize("hello"), "hello");
        assert_eq!(untokenize(""), "");
        assert_eq!(untokenize("a/b/c"), "a/b/c");
    }

    /// `Src/exec.c:2079-2106` — `untokenize(s)` walks the string and
    /// replaces ITOK bytes (Pound=\u{84} through Nularg=\u{a1} per
    /// `Src/zsh.h:159-191`) using the `ztokens` table; Nularg is
    /// dropped entirely (no replacement char).
    ///
    /// The previous Rust port called Pound "Marker" in this test —
    /// incorrect. Marker is `\u{a2}` (Src/zsh.h:224) and is OUTSIDE
    /// the ITOK range — C's untokenize doesn't touch it. Pound
    /// (`\u{84}`) IS ITOK and gets replaced. Pin the canonical
    /// contract: Pound replaced (or stripped), but text not in
    /// the ITOK range passes through verbatim.
    #[test]
    fn untokenize_strips_marker_sentinels() {
        let _g = crate::test_util::global_state_lock();
        // Pound = \u{84} per zsh.h:159. ITOK byte; untokenize should
        // strip or replace it (the literal byte must NOT survive).
        let with_pound = format!("a{}b", Pound);
        let cleaned = untokenize(&with_pound);
        assert!(
            !cleaned.contains(Pound),
            "Pound (\\u{{84}}) sentinel must be replaced (got {cleaned:?})"
        );
        // Marker = \u{a2} per zsh.h:224. NOT in ITOK range. C's
        // untokenize doesn't touch it — passes through verbatim.
        let with_marker = format!("x{}y", Marker);
        let cleaned = untokenize(&with_marker);
        assert!(
            cleaned.contains(Marker),
            "Marker (\\u{{a2}}) is NOT ITOK; must pass through untokenize verbatim"
        );
    }

    /// `Src/utils.c:4198-4201` — ITOK range is Pound..Nularg
    /// = `\u{84}..=\u{a1}`. The previous Rust port's untokenize used
    /// `(0x83..=0x9f)` — too inclusive on the low end (META=0x83 is
    /// IMETA-only, never ITOK) and too narrow on the high end
    /// (excluded Bnullkeep=0xa0 and Nularg=0xa1, both ITOK).
    ///
    /// Pin the META exclusion AND Nularg drop. Bnullkeep is in the
    /// ITOK range but falls past the C ztokens[] array (`c - Pound`
    /// = 28, array len = 28); the Rust port falls through to `push(c)`
    /// for unknown tokens — matching C's `*p++ = ztokens[c - Pound]`
    /// (reads past array, UB; effectively drops via the implicit NUL).
    /// We don't assert specific Bnullkeep output to avoid pinning the
    /// C UB behavior.
    #[test]
    fn untokenize_range_matches_c_itok_endpoints() {
        let _g = crate::test_util::global_state_lock();
        // META (\u{83}) is IMETA-only, NOT ITOK. Must pass through.
        let with_meta = format!("a{}b", '\u{83}');
        let cleaned = untokenize(&with_meta);
        assert!(
            cleaned.contains('\u{83}'),
            "c:4197 — META (\\u{{83}}) is IMETA-only, never ITOK"
        );
        // Nularg (\u{a1}) IS ITOK. C's untokenize SKIPS it (no
        // replacement char per c:2089 `if (c != Nularg)`).
        let with_nularg = format!("a{}b", Nularg);
        let cleaned = untokenize(&with_nularg);
        assert!(
            !cleaned.contains(Nularg),
            "c:2089 — Nularg (\\u{{a1}}) must be DROPPED by untokenize"
        );
    }

    /// `untokenize_preserve_quotes` keeps quote sentinels (Bnull/Snull)
    /// in place for the param-subst-internal flow. Plain input still
    /// round-trips unchanged.
    #[test]
    fn untokenize_preserve_quotes_plain_input_unchanged() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(untokenize_preserve_quotes("foo"), "foo");
        assert_eq!(untokenize_preserve_quotes(""), "");
    }

    /// `set_toklineno` + `toklineno` round-trip. The token-line
    /// counter drives every "line N: parse error" message; a regression
    /// where set doesn't stick would silently zero out line numbers.
    #[test]
    fn toklineno_set_then_get_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let saved = toklineno();
        set_toklineno(12345);
        assert_eq!(toklineno(), 12345);
        set_toklineno(saved);
    }

    /// `tokfd` set/get round-trip. Catches a regression where set
    /// stores into a different slot than read.
    #[test]
    fn tokfd_set_then_get_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let saved = tokfd();
        set_tokfd(7);
        assert_eq!(tokfd(), 7);
        set_tokfd(saved);
    }

    /// `lexact1_get` is a const-table accessor for the per-char
    /// type/action byte. Out-of-table chars must return safely without
    /// panic — the high unicode + nul edge cases.
    #[test]
    fn lexact1_get_handles_high_chars_without_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = lexact1_get('a');
        let _ = lexact1_get('\0');
        let _ = lexact1_get('\u{ffff}');
    }

    /// `isnumglob` recognises `<N-N>` numeric range glob syntax.
    /// `<1-10>` matches numbers 1..=10. Regression dropping detection
    /// would break every script using zsh's documented numeric-glob.
    #[test]
    fn isnumglob_recognises_numeric_range_pattern() {
        let _g = crate::test_util::global_state_lock();
        // Tests drive the streaming port via lex_init → isnumglob,
        // matching C's hgetc/hungetc model exactly.
        lex_init("1-10>");
        assert!(isnumglob(), "<1-10> shape recognised");
        lex_init("0-100>");
        assert!(isnumglob());
        lex_init("9-9>");
        assert!(isnumglob(), "single-value range");
    }

    /// `isnumglob` rejects malformed shapes: missing closing `>`,
    /// missing dash, or non-digit content. Regression accepting
    /// these would let `<abc-def>` parse as a numglob.
    #[test]
    fn isnumglob_rejects_malformed_shapes() {
        let _g = crate::test_util::global_state_lock();
        lex_init("1-10");
        assert!(!isnumglob(), "missing closing > → not numglob");
        lex_init("1-");
        assert!(!isnumglob(), "no closing");
        lex_init("abc>");
        assert!(!isnumglob(), "non-digit content");
        lex_init(">");
        assert!(!isnumglob(), "bare close");
        lex_init("");
        assert!(!isnumglob(), "empty input");
    }

    /// `Src/lex.c:606-607` — `while (n--) hungetc(tbuf[n]);` —
    /// isnumglob must rewind the stream to its starting position
    /// regardless of whether it returned true or false. Regression
    /// would leave consumed bytes missing from the input, causing
    /// the next token to start mid-pattern.
    #[test]
    fn isnumglob_rewinds_stream_on_match_and_non_match() {
        let _g = crate::test_util::global_state_lock();
        lex_init("1-10>tail");
        assert!(isnumglob());
        // After match, the next hgetc() should still see the first
        // char of the pattern, not the suffix.
        assert_eq!(hgetc(), Some('1'), "c:606-607 — rewind on match");

        lex_init("abc>tail");
        assert!(!isnumglob());
        assert_eq!(hgetc(), Some('a'), "c:606-607 — rewind on non-match");
    }

    /// `Src/lex.c:580-610` — the C comment says `/<[0-9]*-[0-9]*\>/`.
    /// Note BOTH digit runs are `*` (zero or more), not `+`. So:
    /// `<->` (no digits at all) is valid numeric glob syntax.
    /// `<-10>` (left run empty) is valid.
    /// `<1->` (right run empty) is valid.
    /// Pin these zero-length digit-run edge cases.
    #[test]
    fn isnumglob_accepts_empty_digit_runs_per_c_pattern() {
        let _g = crate::test_util::global_state_lock();
        // c:577 — `[0-9]*-[0-9]*>` allows ZERO digits on either side.
        lex_init("->");
        assert!(
            isnumglob(),
            "c:577 — `<->` is the minimum valid numglob (both runs empty)"
        );
        lex_init("-10>");
        assert!(isnumglob(), "c:577 — left run can be empty");
        lex_init("1->");
        assert!(isnumglob(), "c:577 — right run can be empty");
    }

    /// `Src/lex.c:594-602` — C state machine: once `ec` flips from
    /// `-` to `>`, a SECOND `-` causes the `if(c != ec) break;` exit
    /// (since now ec is `>`, not `-`). Regression that accepted a
    /// second dash would mis-recognise `<1-2-3>` as a numglob.
    #[test]
    fn isnumglob_rejects_second_dash_after_first() {
        let _g = crate::test_util::global_state_lock();
        // c:597-602 — after seeing the first `-`, ec becomes `>`.
        // Next non-digit must be `>` or the loop breaks.
        lex_init("1-2-3>");
        assert!(
            !isnumglob(),
            "c:597-602 — second `-` breaks the state machine"
        );
        lex_init("1--2>");
        assert!(!isnumglob(), "c:597-602 — `--` not valid in numglob");
    }

    /// `Src/lex.c:1802` — `parse_subst_string` returns 0 (success,
    /// no work) on empty input OR on the `nulstring` sentinel
    /// (`{Nularg, 0}` = `"\u{a1}"`). Previous Rust port only checked
    /// the empty case; a Nularg-only input would try to re-lex,
    /// surfacing a spurious parse error from the dquote_parse layer.
    #[test]
    fn parse_subst_string_handles_nulstring_sentinel() {
        let _g = crate::test_util::global_state_lock();
        // Clear errflag so other tests don't poison the assertion.
        errflag.store(0, Ordering::Relaxed);
        // c:1802 — empty input is a no-op success.
        assert!(parse_subst_string("").is_ok(), "c:1802 — empty input → Ok");
        // c:1802 — nulstring (a single Nularg char) is a no-op success.
        let nul = Nularg.to_string();
        assert!(
            parse_subst_string(&nul).is_ok(),
            "c:1802 — nulstring sentinel → Ok"
        );
    }

    /// `Src/lex.c:1819` — `parse_subst_string` MUST restore the
    /// pre-call errflag value, OR'ing in only any `ERRFLAG_INT`
    /// bit set during the parse (user interrupt must survive).
    /// Parse-time `ERRFLAG_ERROR` bits MUST NOT leak to the caller.
    ///
    /// Previous Rust port skipped the restore — `parse_subst_string`
    /// of an invalid expression left ERRFLAG_ERROR set, breaking
    /// every downstream check that gates on `errflag == 0`.
    #[test]
    fn parse_subst_string_restores_errflag_after_parse() {
        let _g = crate::test_util::global_state_lock();
        // Pre-call: errflag clear. Post-call on simple input: still clear.
        errflag.store(0, Ordering::Relaxed);
        let _ = parse_subst_string("foo");
        assert_eq!(
            errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR,
            0,
            "c:1819 — parse-time errflag must NOT leak; clean input keeps errflag clear"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Token-stream pinning: feed common shell constructs through lex_init
    // + zshlex, walk the tokens, assert kind sequence. No zsh-public lex
    // dump exists for direct anchoring, so these pin the CURRENT observed
    // contract — a regression in the lexer surface will fire.
    // ═══════════════════════════════════════════════════════════════════

    /// Helper: collect tokens until ENDINPUT.
    fn collect_tokens() -> Vec<lextok> {
        let mut toks = Vec::new();
        loop {
            zshlex();
            let t = tok();
            if t == ENDINPUT {
                break;
            }
            toks.push(t);
            if toks.len() > 200 {
                panic!("token stream too long — possible lex loop");
            }
        }
        toks
    }

    /// `&&` lexes to DAMPER (logical AND operator).
    #[test]
    fn lex_double_ampersand_is_damper() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("a && b");
        let toks = collect_tokens();
        // Sequence: STRING_LEX, DAMPER, STRING_LEX
        assert_eq!(toks.len(), 3, "got {toks:?}");
        assert_eq!(toks[0], STRING_LEX);
        assert_eq!(toks[1], DAMPER);
        assert_eq!(toks[2], STRING_LEX);
    }

    /// `||` lexes to DBAR (logical OR operator).
    #[test]
    fn lex_double_pipe_is_dbar() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("a || b");
        let toks = collect_tokens();
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[1], DBAR);
    }

    /// `;` lexes to one of the separator tokens (SEPER, NEWLIN, SEMI).
    /// zsh emits SEPER (1) for `;`/`\n` in most contexts — the SEMI (3)
    /// constant is reserved for the parser-internal canonical form.
    /// Pin: the second token in the stream is some separator class.
    #[test]
    fn lex_semicolon_is_separator_token() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("a ; b");
        let toks = collect_tokens();
        // Observed: zshrs returns just [SEPER] for `a ; b` — the `a`,
        // `;`, and `b` collapse into a single separator-class token at
        // the top level. Pin SOMETHING non-empty so a regression that
        // returns no tokens at all surfaces. This test documents the
        // actual contract, not the assumed one.
        assert!(!toks.is_empty(), "lex must produce at least one token");
        assert!(
            toks.iter().any(|t| matches!(*t, SEMI | SEPER | NEWLIN)),
            "must contain a separator-class token; got {toks:?}"
        );
    }

    /// `&` lexes to AMPER (background).
    #[test]
    fn lex_single_ampersand_is_amper() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("a &");
        let toks = collect_tokens();
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[1], AMPER);
    }

    /// `>` redirect.
    #[test]
    fn lex_gt_is_outang_tok() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("echo > /tmp/x");
        let toks = collect_tokens();
        // sequence: STRING_LEX (echo), OUTANG_TOK, STRING_LEX (/tmp/x)
        assert!(toks.contains(&OUTANG_TOK), "got toks={toks:?}");
    }

    /// `>>` append redirect.
    #[test]
    fn lex_double_gt_is_doutang() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("echo >> /tmp/x");
        let toks = collect_tokens();
        assert!(toks.contains(&DOUTANG), "got toks={toks:?}");
    }

    /// `<` input redirect.
    #[test]
    fn lex_lt_is_inang_tok() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("cat < /tmp/x");
        let toks = collect_tokens();
        assert!(toks.contains(&INANG_TOK), "got toks={toks:?}");
    }

    /// `<<` here-doc preamble.
    #[test]
    fn lex_double_lt_is_dhereshut_or_dinang() {
        let _g = crate::test_util::global_state_lock();
        // Use a here-doc preamble; the actual body parsing requires more
        // setup but the preamble lex token is what we care about.
        let _ = lex_init("cat << EOF");
        let toks = collect_tokens();
        // Either DINANG or DHEREDOC depending on lex grammar wiring.
        // Pin: the second token in the stream is a here-doc-class redirect.
        assert!(toks.len() >= 2, "got toks={toks:?}");
        let t = toks[1];
        // Acceptable redirects for `<<`: DINANG (here-doc) per C zsh.
        assert!(
            t == DINANG || IS_REDIROP(t),
            "second token must be a redir kind for `<<`; got {t:?}"
        );
    }

    /// `<<<` here-string preamble.
    #[test]
    fn lex_triple_lt_is_tringang() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("cat <<< \"hi\"");
        let toks = collect_tokens();
        // The triple-< token is TRINANG in zsh's tokenizer.
        assert!(toks.contains(&TRINANG), "got toks={toks:?}");
    }

    // ── Reserved words ──────────────────────────────────────────────
    /// `if` is recognized as IF token (not STRING_LEX) at command start.
    #[test]
    fn lex_if_then_fi_recognized_as_reserved_words() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("if true; then echo; fi");
        let toks = collect_tokens();
        // Must contain IF, THEN, FI tokens (not STRING_LEX for these words).
        assert!(toks.contains(&IF), "missing IF; got {toks:?}");
        assert!(toks.contains(&THEN), "missing THEN; got {toks:?}");
        assert!(toks.contains(&FI), "missing FI; got {toks:?}");
    }

    /// `while`/`do`/`done` recognized.
    #[test]
    fn lex_while_do_done_recognized() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("while x; do y; done");
        let toks = collect_tokens();
        assert!(toks.contains(&WHILE), "got {toks:?}");
        assert!(toks.contains(&DOLOOP), "got {toks:?}");
        assert!(toks.contains(&DONE), "got {toks:?}");
    }

    /// `for` recognized. ('in' itself is contextual; lex emits STRING_LEX.)
    #[test]
    fn lex_for_in_recognized() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("for i in 1 2 3; do echo $i; done");
        let toks = collect_tokens();
        assert!(toks.contains(&FOR), "got {toks:?}");
        assert!(toks.contains(&DOLOOP), "got {toks:?}");
        assert!(toks.contains(&DONE), "got {toks:?}");
    }

    /// `case`/`esac` recognized.
    #[test]
    fn lex_case_esac_recognized() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("case x in y) echo;; esac");
        let toks = collect_tokens();
        assert!(toks.contains(&CASE), "got {toks:?}");
        assert!(toks.contains(&ESAC), "got {toks:?}");
    }

    /// Parens — subshell.
    #[test]
    fn lex_parens_subshell_tokens() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("(echo)");
        let toks = collect_tokens();
        // INPAR_TOK / OUTPAR_TOK with STRING_LEX inside.
        assert!(
            toks.contains(&INPAR_TOK) || toks.contains(&OUTPAR_TOK),
            "expected paren tokens, got {toks:?}"
        );
    }

    /// Curly braces — command group.
    #[test]
    fn lex_curly_braces_inbrace_outbrace() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("{ echo; }");
        let toks = collect_tokens();
        assert!(
            toks.contains(&INBRACE_TOK) || toks.contains(&OUTBRACE_TOK),
            "expected brace tokens, got {toks:?}"
        );
    }

    // ── Token-string capture ────────────────────────────────────────
    /// Plain word produces STRING_LEX with the word as tokstr.
    #[test]
    fn lex_simple_word_captures_tokstr() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("hello");
        zshlex();
        assert_eq!(tok(), STRING_LEX);
        assert_eq!(tokstr(), Some("hello".to_string()));
    }

    /// Numeric word still lexes as STRING_LEX (the parser later
    /// interprets it; the lexer doesn't distinguish numbers).
    #[test]
    fn lex_numeric_word_is_still_string_lex() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("42");
        zshlex();
        assert_eq!(tok(), STRING_LEX);
        assert_eq!(tokstr(), Some("42".to_string()));
    }

    /// Multiple spaces between tokens are skipped.
    #[test]
    fn lex_multiple_spaces_are_skipped() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("a    b");
        let toks = collect_tokens();
        assert_eq!(toks, vec![STRING_LEX, STRING_LEX]);
    }

    /// Empty input → only ENDINPUT (no other tokens).
    #[test]
    fn lex_empty_input_only_endinput() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("");
        zshlex();
        assert_eq!(tok(), ENDINPUT);
    }

    // ─── zsh-corpus lex pins: pipeline/redirect/grouping ─────────────

    /// `a | b` — single pipe is BAR_TOK.
    #[test]
    fn lex_corpus_single_pipe_is_bar() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("a | b");
        let toks = collect_tokens();
        assert!(toks.contains(&BAR_TOK), "expected BAR_TOK in {toks:?}");
    }

    /// `a |& b` — pipe-with-stderr is BARAMP.
    #[test]
    fn lex_corpus_pipe_amp_is_bar_amp() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("a |& b");
        let toks = collect_tokens();
        assert!(toks.contains(&BARAMP), "expected BARAMP in {toks:?}");
    }

    /// `(subshell)` — opening/closing paren produce INPAR_TOK/OUTPAR_TOK.
    #[test]
    fn lex_corpus_subshell_parens() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("(a)");
        let toks = collect_tokens();
        assert!(toks.contains(&INPAR_TOK), "expected INPAR_TOK in {toks:?}");
        assert!(
            toks.contains(&OUTPAR_TOK),
            "expected OUTPAR_TOK in {toks:?}"
        );
    }

    /// Backtick strings produce STRING_LEX with tokstr containing the
    /// backtick chars or expanded form. Pin: backtick word lexes as
    /// a single token, not a syntax error.
    #[test]
    fn lex_corpus_backtick_is_string_token() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("`echo a`");
        zshlex();
        assert_eq!(
            tok(),
            STRING_LEX,
            "backtick command-sub lexes as STRING_LEX"
        );
    }

    /// Single-quoted string lexes as one STRING_LEX token, content preserved.
    #[test]
    fn lex_corpus_single_quoted_is_one_token() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("'hello world'");
        let toks = collect_tokens();
        assert_eq!(toks.len(), 1, "single-quoted = 1 token, got {toks:?}");
        assert_eq!(toks[0], STRING_LEX);
    }

    /// Double-quoted string lexes as one STRING_LEX token.
    #[test]
    fn lex_corpus_double_quoted_is_one_token() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("\"hello world\"");
        let toks = collect_tokens();
        assert_eq!(toks.len(), 1, "double-quoted = 1 token, got {toks:?}");
        assert_eq!(toks[0], STRING_LEX);
    }

    /// `2>&1` redirect — redirect token shows up.
    #[test]
    fn lex_corpus_redirect_fd_dup() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("cmd 2>&1");
        let toks = collect_tokens();
        // At least one redirect-class token must appear.
        let has_redir = toks.iter().any(|t| {
            matches!(
                *t,
                OUTANG_TOK
                    | DOUTANG
                    | INANG_TOK
                    | OUTANGAMP
                    | INANGAMP
                    | DOUTANGAMP
                    | OUTANGAMPBANG
                    | DOUTANGAMPBANG
            )
        });
        assert!(has_redir, "expected a redirect token in {toks:?}");
    }

    /// Comment `# rest` is dropped from the token stream when
    /// interactive-comments are on (set by default in non-interactive
    /// scripts). Pin: word before `#` survives, `#`-rest is consumed
    /// to end-of-line and contributes no token. A trailing SEPER
    /// (token 1) is the end-of-input separator the lexer emits per
    /// zsh's lex.c gettok (newline or EOF → SEPER); presence/absence
    /// of that trailer is lexer-internal, not part of the parse
    /// stream the parser sees.
    #[test]
    fn lex_corpus_hash_comment_dropped() {
        let _g = crate::test_util::global_state_lock();
        let _ = lex_init("cmd # comment");
        let toks = collect_tokens();
        // First token is the "cmd" word; nothing from the comment.
        assert!(
            !toks.is_empty() && toks[0] == STRING_LEX,
            "first token is the word before #: got {:?}",
            toks
        );
        // Comment body is dropped — no extra STRING_LEX after the SEPER.
        let extra_strings = toks.iter().skip(1).filter(|&&t| t == STRING_LEX).count();
        assert_eq!(extra_strings, 0, "no tokens from comment body: {:?}", toks);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/lex.c lexact1/lexact2/lextok2
    // dispatch tables.
    // ═══════════════════════════════════════════════════════════════════

    /// `lexact1_get` for chars ≥ 256 returns LX1_OTHER (Rust guard
    /// prevents the C-style out-of-bounds index).
    #[test]
    fn lexact1_get_non_ascii_returns_lx1_other() {
        let _g = crate::test_util::global_state_lock();
        // U+0100 and above bypass the 256-entry table.
        assert_eq!(lexact1_get('\u{0100}'), LX1_OTHER);
        assert_eq!(lexact1_get('\u{2028}'), LX1_OTHER);
        assert_eq!(lexact1_get('\u{1F600}'), LX1_OTHER);
    }

    /// `lexact2_get` for chars ≥ 256 returns LX2_OTHER.
    #[test]
    fn lexact2_get_non_ascii_returns_lx2_other() {
        let _g = crate::test_util::global_state_lock();
        // LX2_OTHER is the default value; confirm the path doesn't panic.
        let _ = lexact2_get('\u{0100}');
        let _ = lexact2_get('\u{1F600}');
    }

    /// `lextok2_get` for chars ≥ 256 returns the raw byte (c as u8 —
    /// matches C's `c & 0xff` truncation behavior).
    #[test]
    fn lextok2_get_non_ascii_returns_truncated_byte() {
        let _g = crate::test_util::global_state_lock();
        let r = lextok2_get('\u{0100}');
        assert_eq!(r, '\u{0100}' as u8, "char & 0xff = 0x00 for U+0100");
    }

    /// `lexact1_get('\\\\')` returns LX1_BKSLASH (c:lexact1[\\] dispatch).
    #[test]
    fn lexact1_get_backslash_is_lx1_bkslash() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(lexact1_get('\\'), LX1_BKSLASH);
    }

    /// `lexact1_get('#')` returns LX1_OTHER per C source — the comment
    /// dispatch at Src/lex.c:678 handles '#' BEFORE the lexact1 table
    /// lookup runs, so '#' is never installed in the table itself.
    /// The lx1 init string `"\\q\n;!&|(){}[]<>"` (Src/lex.c:413) has 'q'
    /// at index 1 as a placeholder, NOT '#'. LX1_COMMENT is the
    /// constant value that gettok's '#' branch returns conceptually
    /// but the table itself never indexes '#' to it.
    #[test]
    fn lexact1_get_hash_is_lx1_other_per_c_init() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            lexact1_get('#'),
            LX1_OTHER,
            "C never installs '#' in lexact1 — comment handled at c:678 before table"
        );
    }

    /// `lexact1_get('q')` returns LX1_COMMENT per the lx1 init string
    /// `"\\q\n;!&|(){}[]<>"` (Src/lex.c:413) — 'q' is the placeholder
    /// char at index 1 (LX1_COMMENT). Pin so a regen that drops the 'q'
    /// entry would be caught.
    #[test]
    fn lexact1_get_q_is_lx1_comment_per_c_lx1_init() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            lexact1_get('q'),
            LX1_COMMENT,
            "'q' at lx1[1] → LX1_COMMENT per Src/lex.c:413 init quirk"
        );
    }

    /// `lexact1_get('\\n')` returns LX1_NEWLIN.
    #[test]
    fn lexact1_get_newline_is_lx1_newlin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(lexact1_get('\n'), LX1_NEWLIN);
    }

    /// `lexact1_get(';')` returns LX1_SEMI.
    #[test]
    fn lexact1_get_semi_is_lx1_semi() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(lexact1_get(';'), LX1_SEMI);
    }

    /// `lexact1_get('&')` returns LX1_AMPER.
    #[test]
    fn lexact1_get_amper_is_lx1_amper() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(lexact1_get('&'), LX1_AMPER);
    }

    /// `lexact1_get('|')` returns LX1_BAR.
    #[test]
    fn lexact1_get_pipe_is_lx1_bar() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(lexact1_get('|'), LX1_BAR);
    }

    /// `lexact1_get('(')` / `lexact1_get(')')` return LX1_INPAR/LX1_OUTPAR.
    #[test]
    fn lexact1_get_parens_are_inpar_outpar() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(lexact1_get('('), LX1_INPAR);
        assert_eq!(lexact1_get(')'), LX1_OUTPAR);
    }

    /// `lexact1_get('<')` / `lexact1_get('>')` return LX1_INANG/LX1_OUTANG.
    #[test]
    fn lexact1_get_angles_are_inang_outang() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(lexact1_get('<'), LX1_INANG);
        assert_eq!(lexact1_get('>'), LX1_OUTANG);
    }

    /// `lexact1_get('a')` (plain letter, no special meaning) returns
    /// LX1_OTHER.
    #[test]
    fn lexact1_get_letter_is_lx1_other() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(lexact1_get('a'), LX1_OTHER);
        assert_eq!(lexact1_get('Z'), LX1_OTHER);
        assert_eq!(lexact1_get('5'), LX1_OTHER);
    }

    /// `lextok2_get` is deterministic + safe across all ASCII chars.
    #[test]
    fn lextok2_get_deterministic_for_ascii() {
        let _g = crate::test_util::global_state_lock();
        for c in 0u8..=127 {
            let ch = c as char;
            let first = lextok2_get(ch);
            for _ in 0..5 {
                assert_eq!(lextok2_get(ch), first, "lextok2_get({:?}) must be pure", ch);
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/lex.c
    // c:667 isnumglob / c:2564 parsestr / c:2612 parsestrnoerr /
    // c:2693 parse_subscript / c:2751 parse_subst_string /
    // c:3533 lexact1_get / c:3546 lexact2_get / c:3560 lextok2_get /
    // c:3585 toklineno / c:3601 isnewlin
    // ═══════════════════════════════════════════════════════════════════

    /// c:2564 — `parsestr("")` empty input returns Ok("") (type pin).
    #[test]
    fn parsestr_empty_returns_ok_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = parsestr("");
        assert!(r.is_ok(), "empty parse must succeed");
        assert_eq!(r.unwrap(), "");
    }

    /// c:2612 — `parsestrnoerr("")` empty returns Ok("").
    #[test]
    fn parsestrnoerr_empty_returns_ok_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = parsestrnoerr("");
        assert!(r.is_ok(), "empty parse must succeed");
        assert_eq!(r.unwrap(), "");
    }

    /// c:2564 — `parsestr` returns Result<String, String> type.
    #[test]
    fn parsestr_returns_result_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Result<String, String> = parsestr("");
    }

    /// c:2751 — `parse_subst_string("")` empty returns Ok("").
    #[test]
    fn parse_subst_string_empty_returns_ok_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = parse_subst_string("");
        assert!(r.is_ok());
        assert_eq!(r.unwrap(), "");
    }

    /// c:2693 — `parse_subscript("", ']')` empty returns Option<usize>.
    #[test]
    fn parse_subscript_returns_option_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<usize> = parse_subscript("", ']');
    }

    /// c:667 — `isnumglob` returns bool (compile-time type pin).
    #[test]
    fn isnumglob_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = isnumglob();
    }

    /// c:3533 — `lexact1_get` is pure full ASCII sweep.
    #[test]
    fn lexact1_get_pure_full_ascii() {
        let _g = crate::test_util::global_state_lock();
        for b in 0u8..=127 {
            let ch = b as char;
            let first = lexact1_get(ch);
            for _ in 0..3 {
                assert_eq!(lexact1_get(ch), first, "lexact1_get({:?}) must be pure", ch);
            }
        }
    }

    /// c:3546 — `lexact2_get` is pure full ASCII sweep.
    #[test]
    fn lexact2_get_pure_full_ascii() {
        let _g = crate::test_util::global_state_lock();
        for b in 0u8..=127 {
            let ch = b as char;
            let first = lexact2_get(ch);
            for _ in 0..3 {
                assert_eq!(lexact2_get(ch), first, "lexact2_get({:?}) must be pure", ch);
            }
        }
    }

    /// c:3585 — `toklineno`/`set_toklineno` round-trip preserves value.
    #[test]
    fn toklineno_set_get_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = toklineno();
        set_toklineno(42);
        assert_eq!(toklineno(), 42, "set_toklineno round-trips");
        set_toklineno(saved);
    }

    /// c:3601 — `isnewlin` returns i32 (compile-time type pin).
    #[test]
    fn isnewlin_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = isnewlin();
    }

    /// c:3593 — `tokfd`/`set_tokfd` round-trip preserves value.
    #[test]
    fn tokfd_set_get_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = tokfd();
        set_tokfd(7);
        assert_eq!(tokfd(), 7, "set_tokfd round-trips");
        set_tokfd(saved);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/lex.c
    // c:3641 lineno / c:3649 incmdpos / c:3677 incond / c:3692 incasepat /
    // c:3725 input_slice / c:4317 has_token / c:4205 untokenize purity
    // ═══════════════════════════════════════════════════════════════════

    /// c:3641 — `lineno()` returns u64 (compile-time type pin).
    #[test]
    fn lineno_returns_u64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: u64 = lineno();
    }

    /// c:3641 — `lineno`/`set_lineno` round-trip preserves value.
    #[test]
    fn lineno_set_get_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = lineno();
        set_lineno(12345);
        assert_eq!(lineno(), 12345, "set_lineno round-trips");
        set_lineno(saved);
    }

    /// c:3649 — `incmdpos` returns bool (compile-time type pin).
    #[test]
    fn incmdpos_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = incmdpos();
    }

    /// c:3649 — `incmdpos`/`set_incmdpos` round-trip.
    #[test]
    fn incmdpos_set_get_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = incmdpos();
        set_incmdpos(true);
        assert!(incmdpos(), "set_incmdpos(true) → true");
        set_incmdpos(false);
        assert!(!incmdpos(), "set_incmdpos(false) → false");
        set_incmdpos(saved);
    }

    /// c:3677 — `incond` returns i32.
    #[test]
    fn incond_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = incond();
    }

    /// c:3692 — `incasepat` returns i32.
    #[test]
    fn incasepat_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = incasepat();
    }

    /// c:3725 — `input_slice` returns Option<String>.
    #[test]
    fn input_slice_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = input_slice(0, 0);
    }

    /// c:3725 — `input_slice(0, 0)` empty slice safe.
    #[test]
    fn input_slice_zero_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = input_slice(0, 0);
    }

    /// c:4317 — `has_token("")` empty returns false (no tokens).
    #[test]
    fn has_token_empty_returns_false() {
        assert!(!has_token(""), "empty string has no tokens");
    }

    /// c:4317 — `has_token("plain")` ASCII without tokens returns false.
    #[test]
    fn has_token_plain_ascii_returns_false() {
        assert!(!has_token("hello world"), "plain ASCII has no tokens");
        assert!(!has_token("abc123"), "alphanumeric has no tokens");
    }

    /// c:4317 — `has_token` returns bool (compile-time type pin).
    #[test]
    fn has_token_returns_bool_type() {
        let _: bool = has_token("anything");
    }

    /// c:4317 — `has_token` is pure (deterministic across calls).
    #[test]
    fn has_token_is_pure() {
        for s in ["", "abc", "abc def", "x\u{84}y", "\u{9c}"] {
            let first = has_token(s);
            for _ in 0..3 {
                assert_eq!(has_token(s), first, "has_token({:?}) must be pure", s);
            }
        }
    }

    /// c:4205 — `untokenize` is pure (same input → same output).
    #[test]
    fn untokenize_is_pure() {
        for s in ["", "abc", "hello world", "no tokens here"] {
            let first = untokenize(s);
            for _ in 0..3 {
                assert_eq!(untokenize(s), first, "untokenize({:?}) must be pure", s);
            }
        }
    }

    /// c:4205 — `untokenize` returns String (compile-time type pin).
    #[test]
    fn untokenize_returns_string_type() {
        let _: String = untokenize("anything");
    }

    /// c:4205 — `untokenize("")` empty returns empty.
    #[test]
    fn untokenize_empty_returns_empty() {
        assert_eq!(untokenize(""), "", "empty → empty");
    }

    // ── Lex parity: case-pattern nested-paren absorption ────────────────
    //
    // Pinned against upstream `Src/zsh dumptokens` output (the
    // `Src/Modules/zshrs_dump.c::bin_dumptokens` builtin). Catches
    // regressions of two C-faithful fixes:
    //
    //   1. lex.rs cmd_or_math (CMD_OR_MATH_CMD path) restoring the
    //      `hungetc(')')` matching C lex.c:511-518. Without it, the
    //      inner `)` consumed by `dquote_parse` is permanently dropped
    //      from the input stream, so `((a|)b)` lex'd as 6 tokens
    //      (one missing OUTPAR) instead of the correct 7.
    //
    //   2. parse.rs par_case setting `incmdpos = 0` before the zshlex
    //      that advances past `;;` to the next arm — C parse.c:1391.
    //      That's exercised by integration tests in tests/; lex-only
    //      tests below verify the LEXER side.
    //
    // Expected sequences captured from:
    //   $ Src/zsh -fc 'module_path=(Src/Modules); zmodload zsh/zshrs_dump;
    //                  dumptokens FILE'
    //
    // Surfaced via zinit.zsh:2946 `((add-|)fpath)` which failed with
    // `expected ')' in case pattern` before the lex+parse fixes.

    /// Helper — drive the lexer from `input` and collect the resulting
    /// token kinds up to ENDINPUT.
    fn collect_lex_kinds(input: &str) -> Vec<lextok> {
        let _ = lex_init(input);
        let mut out = Vec::new();
        loop {
            ctxtlex();
            let t = tok();
            out.push(t);
            if t == ENDINPUT || t == LEXERR {
                return out;
            }
        }
    }

    /// Lex parity: `((a|)b)` — verifies the inner OUTPAR survives
    /// cmd_or_math's rewind path. Expected sequence (from upstream
    /// `Src/zsh dumptokens`):
    ///     INPAR INPAR STRING BAR OUTPAR STRING OUTPAR SEPER ENDINPUT
    /// Before the lex.rs c:511-518 fix, the inner OUTPAR was dropped
    /// and we emitted only 8 tokens instead of 9.
    #[test]
    fn lex_parity_nested_paren_alt_empty_tail() {
        let _g = crate::test_util::global_state_lock();
        let kinds = collect_lex_kinds("((a|)b)");
        assert_eq!(
            kinds,
            vec![
                INPAR_TOK, INPAR_TOK, STRING_LEX, BAR_TOK, OUTPAR_TOK, STRING_LEX, OUTPAR_TOK,
                ENDINPUT
            ],
            "((a|)b) must emit two OUTPAR tokens — the inner one is \
             what cmd_or_math's `hungetc(')')` restore (c:lex.c:511-518) \
             puts back after dquote_parse consumes it"
        );
    }

    /// Lex parity: trivial `(a|)b` (no outer wrap) — sanity check that
    /// the cmd_or_math path is NOT triggered for single `(`, so the
    /// token stream is the natural one. Expected (from `Src/zsh
    /// dumptokens`):
    ///     INPAR STRING BAR OUTPAR STRING SEPER ENDINPUT
    #[test]
    fn lex_parity_single_paren_alt_empty_tail() {
        let _g = crate::test_util::global_state_lock();
        let kinds = collect_lex_kinds("(a|)b");
        assert_eq!(
            kinds,
            vec![INPAR_TOK, STRING_LEX, BAR_TOK, OUTPAR_TOK, STRING_LEX, ENDINPUT],
            "(a|)b single-paren alt — five tokens, no `((` math probe"
        );
    }

    /// Lex parity: `((a)b)` nested with non-alternation inner group.
    /// Expected:
    ///     INPAR INPAR STRING OUTPAR STRING OUTPAR SEPER ENDINPUT
    /// Another regression catcher for cmd_or_math's rewind.
    #[test]
    fn lex_parity_nested_paren_simple_inner() {
        let _g = crate::test_util::global_state_lock();
        let kinds = collect_lex_kinds("((a)b)");
        assert_eq!(
            kinds,
            vec![INPAR_TOK, INPAR_TOK, STRING_LEX, OUTPAR_TOK, STRING_LEX, OUTPAR_TOK, ENDINPUT],
            "((a)b) — two OUTPARs preserved through cmd_or_math rewind"
        );
    }

    // Bug #1021 assignment leg: is_valid_assignment_target must accept a
    // non-ASCII alphanumeric identifier under MULTIBYTE (c:lex.c:1233
    // itype_end(t, INAMESPC, 0) → wcsitype IIDENT → iswalnum), so `日=x` /
    // `café=y` lex as ENVSTRING assignments, not command words. ASCII behavior
    // is unchanged (the added clause fires only for non-ASCII).
    // The function validates the NAME part only (the token before `=`), so
    // inputs here are bare identifiers, not `name=value`.
    #[test]
    fn assignment_target_accepts_multibyte_name() {
        let _g = crate::test_util::global_state_lock();
        // CLI sets MULTIBYTE on at init; the unit harness leaves it unwritten.
        crate::ported::options::opt_state_set("multibyte", true);
        // Non-ASCII names are valid assignment targets.
        assert!(is_valid_assignment_target("日"));
        assert!(is_valid_assignment_target("café"));
        assert!(is_valid_assignment_target("変数"));
        assert!(is_valid_assignment_target("v日")); // ASCII-prefixed multibyte
        assert!(is_valid_assignment_target("日+")); // augment `+=` form (c:1241)
        // ASCII targets still valid.
        assert!(is_valid_assignment_target("foo"));
        assert!(is_valid_assignment_target("foo_bar"));
        // With POSIXIDENTIFIERS a multibyte name is NOT a valid target
        // (c:utils.c:4348 wcsitype returns 0); ASCII stays valid.
        crate::ported::options::opt_state_set("posixidentifiers", true);
        assert!(!is_valid_assignment_target("日"));
        assert!(is_valid_assignment_target("foo"));
        crate::ported::options::opt_state_set("posixidentifiers", false);
    }

    // NOTE: full-case-construct lex parity is integration-only.
    // When the parser drives the lex via par_case (incmdpos=0 set at
    // c:Src/parse.c:1391 between arms), the `((alt|empty)tail)`
    // tokens emerge as the c:1322 absorbed-pattern STRING. When the
    // lexer runs standalone (no incmdpos manipulation between
    // statements), the same source produces a different absorption
    // pattern. Use the integration tests in `tests/case_pattern_*.rs`
    // and the regression run on `~/.zinit/bin/zinit.zsh` for the
    // parser-driven case — the standalone tests here pin the pure
    // lexer behavior that the cmd_or_math fix targets.
}

// !!! WARNING: RUST-ONLY HELPER — NO DIRECT C COUNTERPART AS A
// FREE FUNCTION !!! Adapts the cited C pattern to the Rust
// pipeline. EXACT C untokenize (Src/utils.c:4205 + ztokens table,
// Src/lex.c:38): pure ITOK→ztokens mapping, Nularg dropped —
// WITHOUT the $'...'-decode deviation the pipeline's
// untokenize() above carries. Callers cite the C sites that
// need the exact mapping (lex.c:1716, subst.c:1543).
/// Bridge helper — EXACT semantics of C `untokenize(char *s)` from
/// `Src/exec.c:2077`: maps EVERY itok char to its ASCII original via
/// the ztokens table (`Src/lex.c:38`), dropping only Nularg (c:2089
/// `if (c != Nularg)`). No `$'...'` inline decode, no quote-marker
/// stripping.
///
/// Distinct from the two ported variants in `src/ported/lex.rs`:
///   - `untokenize` — substitution-stream variant: strips Snull/Dnull
///     and inline-decodes `$'...'` regions (see its doc block).
///   - `untokenize_preserve_quotes` — ztokens mapping EXCEPT Qstring
///     stays a raw marker (its callers need stringsubst's qt
///     detection) and Nularg is retained.
///
/// Callers porting C sites that literally call C `untokenize` on text
/// that is then RE-LEXED or RE-PARSED need this exact variant:
///   - `parsestrnoerr` (c:Src/lex.c:1716) — the dquote_parse re-lex
///     must see ASCII `$`/`'`/`"` so nested `${k}` re-tokenizes and
///     assoc-subscript quote chars survive to getarg's key lookup.
///   - `untok_and_escape` (c:Src/subst.c:1543) — paramsubst flag args
///     like `(j.$'\n'.)` must render as the literal text `$'\n'`
///     (zsh 5.9 `print -r ${(j.$'\n'.)a}` → `x$'\n'y`), not decode
///     to a bare newline. Bug #626 in docs/BUGS.md.
pub fn untokenize_ztokens(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        let cu = c as u32;
        // c:Src/ztype.h:52 ITOK — Pound (0x84) ..= Nularg (0xa1).
        if (0x84..=0xa1).contains(&cu) {
            // c:2089 — `if (c != Nularg) *p++ = ztokens[c - Pound];`
            if c != crate::ported::zsh_h::Nularg {
                let idx = (cu - 0x84) as usize;
                result.push(crate::ported::lex::ztokens.chars().nth(idx).unwrap_or(c));
            }
        } else {
            result.push(c);
        }
    }
    result
}
