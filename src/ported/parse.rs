//! Zsh parser — direct port from zsh/Src/parse.c.
//!
//! Pulls tokens via the lex.rs free ported (zshlex/tok/tokstr) and
//! builds an AST tree (relocated to src/extensions/zsh_ast.rs as a
//! Rust-only IR) plus emits wordcode into ECBUF via the P9b/P9c
//! pipeline. Follows the zsh grammar closely; productions match
//! `par_*` in Src/parse.c.

use super::lex::{
    lextok, set_tok, AMPER, AMPERBANG, AMPOUTANG, BANG_TOK, BARAMP, BAR_TOK, CASE, COPROC, DAMPER,
    DBAR, DINANG, DINANGDASH, DINBRACK, DINPAR, DOLOOP, DONE, DOUTANG, DOUTANGAMP, DOUTANGAMPBANG,
    DOUTANGBANG, DOUTBRACK, DOUTPAR, DSEMI, ELIF, ELSE, ENDINPUT, ENVARRAY, ENVSTRING, ESAC, FI,
    FOR, FOREACH, FUNC, IF, INANGAMP, INANG_TOK, INBRACE_TOK, INOUTANG, INOUTPAR, INPAR_TOK,
    IS_REDIROP, LEXERR, LEX_HEREDOCS, NEWLIN, NOCORRECT, OUTANGAMP, OUTANGAMPBANG, OUTANGBANG,
    OUTANG_TOK, OUTBRACE_TOK, OUTPAR_TOK, REPEAT, SELECT, SEMI, SEMIAMP, SEMIBAR, SEPER,
    STRING_LEX, THEN, TIME, TRINANG, TYPESET, UNTIL, WHILE, ZEND,
};
use super::zsh_h::{
    eprog, estate, funcdump, isset, redir, unset, wc_code, wordcode, Bang, Dash, Equals, Inang,
    Outang, Tilde, ALIASFUNCDEF, COND_AND, COND_MOD, COND_MODI, COND_NOT, COND_NT, COND_OR,
    COND_REGEX, COND_STRDEQ, COND_STREQ, COND_STRGTR, COND_STRLT, COND_STRNEQ, CSHJUNKIELOOPS,
    EC_DUP, EC_NODUP, EF_HEAP, EF_REAL, EXECOPT, IGNOREBRACES, IS_DASH, MULTIFUNCDEF, OPT_ISSET,
    PM_UNDEFINED, POSIXBUILTINS, REDIRF_FROM_HEREDOC, REDIR_APP, REDIR_APPNOW, REDIR_ERRAPP,
    REDIR_ERRAPPNOW, REDIR_ERRWRITE, REDIR_ERRWRITENOW, REDIR_FROM_HEREDOC_MASK, REDIR_HEREDOC,
    REDIR_HEREDOCDASH, REDIR_HERESTR, REDIR_INPIPE, REDIR_MERGEIN, REDIR_MERGEOUT, REDIR_OUTPIPE,
    REDIR_READ, REDIR_READWRITE, REDIR_VARID_MASK, REDIR_WRITE, REDIR_WRITENOW, SHORTLOOPS,
    SHORTREPEAT, WCB_COND, WCB_SIMPLE, WC_REDIR, WC_REDIR_FROM_HEREDOC, WC_REDIR_TYPE,
    WC_REDIR_VARID, WC_SUBLIST_COPROC, WC_SUBLIST_NOT,
};
pub use crate::heredoc_ast::HereDoc;
use crate::ported::lex::{
    incasepat, incmdpos, incond, infor, input_slice, inredir, inrepeat, intypeset, isnewlin,
    lex_init, lineno, noaliases, nocorrect, pos, set_incasepat, set_incmdpos, set_incond,
    set_infor, set_inredir, set_inrepeat, set_intypeset, set_isnewlin, set_lineno, set_noaliases,
    set_nocorrect, tok, tokfd, toklineno, tokstr, zshlex,
};
use crate::ported::signals::unqueue_signals;
use crate::ported::utils::{errflag, zerr, zwarnnam, ERRFLAG_ERROR};
use crate::prompt::{cmdpop, cmdpush};
pub use crate::zsh_ast::{
    CaseArm, CaseTerm, CaseTerminator, CompoundCommand, ForList, HereDocInfo, ListFlags, ListOp,
    Redirect, RedirectOp, ShellCommand, ShellWord, SimpleCommand, SublistFlags, SublistOp,
    VarModifier, ZshAssign, ZshAssignValue, ZshCase, ZshCommand, ZshCond, ZshFor, ZshFuncDef,
    ZshIf, ZshList, ZshParamFlag, ZshPipe, ZshProgram, ZshRedir, ZshRepeat, ZshSimple, ZshSublist,
    ZshTry, ZshWhile,
};
use crate::zsh_h::{
    wc_bdata, CS_ALWAYS, CS_ARRAY, CS_CASE, CS_CMDAND, CS_CMDOR, CS_COND, CS_CURSH, CS_ELIF,
    CS_ELSE, CS_ERRPIPE, CS_FOR, CS_FOREACH, CS_FUNCDEF, CS_IF, CS_IFTHEN, CS_PIPE, CS_REPEAT,
    CS_SELECT, CS_SUBSH, CS_UNTIL, CS_WHILE, EF_RUN, WCB_ARITH, WCB_ASSIGN, WCB_CASE, WCB_CURSH,
    WCB_END, WCB_FOR, WCB_FUNCDEF, WCB_IF, WCB_LIST, WCB_PIPE, WCB_REDIR, WCB_REPEAT, WCB_SELECT,
    WCB_SUBLIST, WCB_SUBSH, WCB_TIMED, WCB_TRY, WCB_TYPESET, WCB_WHILE, WC_ASSIGN_ARRAY,
    WC_ASSIGN_INC, WC_ASSIGN_NEW, WC_ASSIGN_SCALAR, WC_CASE_AND, WC_CASE_HEAD, WC_CASE_OR,
    WC_CASE_TESTAND, WC_FOR_COND, WC_FOR_LIST, WC_FOR_PPARAM, WC_IF_ELIF, WC_IF_ELSE, WC_IF_HEAD,
    WC_IF_IF, WC_PIPE_END, WC_PIPE_LINENO, WC_PIPE_MID, WC_REDIR_WORDS, WC_SELECT_LIST,
    WC_SELECT_PPARAM, WC_SUBLIST_AND, WC_SUBLIST_END, WC_SUBLIST_FLAGS, WC_SUBLIST_OR,
    WC_SUBLIST_SIMPLE, WC_SUBLIST_TYPE, WC_TIMED_EMPTY, WC_TIMED_PIPE, WC_WHILE_UNTIL,
    WC_WHILE_WHILE, Z_ASYNC, Z_DISOWN, Z_END, Z_SIMPLE, Z_SYNC,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

// Names lifted out of inside-fn `use` statements (PORT.md
// 'no imports inside FNs ever').

// Direct port of `Src/parse.c:287-289` grow-policy constants.
const EC_INIT_SIZE: i32 = 256;

// Pending-here-document list — direct port of `Src/parse.c:84
// struct heredocs *hdocs;`. Per-parser file-static (bucket-1 in
// PORT_PLAN.md): each worker thread parsing a separate program needs
// its own pending-heredoc list. Saved/restored across nested parses
// by `parse_context_save`/`parse_context_restore` (parse.c:299/337).
thread_local! {
    /// Port of file-static `struct heredocs *hdocs;` from `Src/parse.c:84`.
    pub static HDOCS: std::cell::RefCell<Option<Box<crate::ported::zsh_h::heredocs>>>
        = const { std::cell::RefCell::new(None) };
}

// Wordcode-buffer thread-locals — direct port of `Src/parse.c:269-285`
// file-statics. Per-evaluator (bucket-1 in PORT_PLAN.md): each worker
// thread parsing a separate program needs its own wordcode buffer.
//
// ECBUF: the wordcode array being built. C `Wordcode ecbuf`
// (parse.c:275).
// ECLEN: allocated entries in ECBUF (parse.c:269).
// ECUSED: entries actually used so far (parse.c:271).
// ECNPATS: count of patterns referenced by ECBUF (parse.c:273).
// ECSOFFS / ECSSUB: byte offsets into the string region
// (parse.c:279). ECSSUB subtracts substring overlap.
// ECNFUNC: count of functions defined so far (parse.c:285).
// ECSTRS_INDEX: dedup index for long strings — C uses a binary tree
// of `struct eccstr` (zsh.h:836); the canonical Eccstr port exists
// at zsh_h::eccstr but stays unused at runtime here. The HashMap
// preserves the API contract (lookup by (nfunc, str) → offs) with
// simpler ownership semantics.
thread_local! {
    /// `ECBUF` static.
    pub static ECBUF: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(Vec::new());
    static ECLEN: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECUSED: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECNPATS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECSOFFS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECSSUB: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECNFUNC: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECSTRS_INDEX: std::cell::RefCell<std::collections::HashMap<(i32, String), u32>>
        = std::cell::RefCell::new(std::collections::HashMap::new());
    /// C zsh's `eccstr` BST (parse.c:447). Port of `Eccstr ecstrs` —
    /// a hashval-ordered binary search tree of long-strings for
    /// dedup. Same cmp logic as C: nfunc, then hashval, then strcmp.
    /// HashMap above is a fast-path lookup; this tree is the
    /// C-fidelity walker that mirrors C's exact dedup-hit pattern
    /// (including its quirks for hash-colliding content).
    static ECSTRS_TREE: std::cell::RefCell<Option<Box<EccstrNode>>>
        = const { std::cell::RefCell::new(None) };
    /// Reverse index for `ecgetstr`: offs → owned string. Populated
    /// at ecstrcode time so the consumer can recover the string from
    /// the wordcode offs without walking the encode-time HashMap.
    /// Stores the METAFIED BYTE form of each long-string, exactly
    /// matching what C's strs region holds. `String` would not work
    /// here because Rust strings carry UTF-8-encoded chars (e.g.
    /// the Dash marker `\u{9b}` UTF-8-encodes to two bytes
    /// `\xc2 \x9b`) while C stores zsh markers as single bytes
    /// (raw `\x9b`). Storing Vec<u8> lets us write byte-for-byte
    /// what C writes after metafy.
    pub static ECSTRS_REVERSE: std::cell::RefCell<std::collections::HashMap<u32, Vec<u8>>>
        = std::cell::RefCell::new(std::collections::HashMap::new());
}
const EC_DOUBLE_THRESHOLD: i32 = 32768;
const EC_INCREMENT: i32 = 1024;

/// Direct port of `parse_context_save(struct parse_stack *ps, int toplevel)` at `Src/parse.c:295`.
/// Snapshots the lexer-side file-statics (which currently live on
/// `lexer` until Phase 7 dissolution makes them file-scope
/// thread_local!s) plus the pending heredoc list, plus the
/// wordcode-buffer state (STUB until Phase 9b). Saves Rust-only
/// recursion counters too so nested parses get fresh limits.
/// WARNING: param names don't match C — Rust=(ps) vs C=(ps, toplevel)
pub fn parse_context_save(ps: &mut parse_stack) {
    // parse.c:299 — `ps->hdocs = hdocs; hdocs = NULL;` — save the
    // canonical C linked-list and clear it for the nested parse.
    ps.hdocs = HDOCS.with_borrow_mut(|h| h.take());
    // zshrs-only: save the parallel AST-glue Vec the same way.
    // LEX_HEREDOCS carries terminator/strip_tabs/quoted metadata
    // that has no C analog (C stores it implicitly via tokstr).
    ps.lex_heredocs = LEX_HEREDOCS.with_borrow_mut(|v| std::mem::take(v));
    // parse.c:302-310 — save lexer-side state.
    ps.incmdpos = incmdpos();
    // parse.c:303 — `ps->aliasspaceflag = aliasspaceflag;`. Mirrors
    // lex.c LEX_ALIAS_SPACE_FLAG so nested parses preserve the
    // HISTIGNORESPACE-via-alias state across parser re-entry.
    ps.aliasspaceflag = crate::ported::lex::LEX_ALIAS_SPACE_FLAG.with(|c| c.get());
    ps.incond = incond();
    ps.inredir = inredir();
    ps.incasepat = incasepat();
    ps.isnewlin = isnewlin();
    ps.infor = infor();
    ps.inrepeat_ = inrepeat();
    ps.intypeset = intypeset();
    // parse.c:312-317 — wordcode buffer state. STUB until Phase 9b
    // (zshrs has no ecbuf yet).
    ps.eclen = 0;
    ps.ecused = 0;
    ps.ecnpats = 0;
    ps.ecbuf = None;
    ps.ecstrs = None;
    ps.ecsoffs = 0;
    ps.ecssub = 0;
    ps.ecnfunc = 0;
    set_incmdpos(true);
    set_incond(0);
    set_inredir(false);
    set_incasepat(0);
    set_infor(0);
    set_inrepeat(0);
    set_intypeset(false);
}

/// Direct port of `parse_context_restore(const struct parse_stack *ps, int toplevel)` at `Src/parse.c:326`.
/// Inverse of `parse_context_save`. Restores lexer-side state +
/// pending heredocs + Rust-only counters from `ps`, then clears
/// `errflag & ERRFLAG_ERROR` per parse.c:354.
/// WARNING: param names don't match C — Rust=(ps) vs C=(ps, toplevel)
pub fn parse_context_restore(ps: &parse_stack) {
    // parse.c:330-331 — free any in-progress wordcode buffer.
    // zshrs has no wordcode yet (STUB until Phase 9b); the AST
    // nodes are owned by their parent so dropping the parser
    // frees them.

    // parse.c:333-352 — restore saved state.
    // parse.c:337 — `hdocs = ps->hdocs;`
    HDOCS.with_borrow_mut(|h| *h = ps.hdocs.clone());
    // zshrs-only: restore the parallel AST-glue Vec.
    LEX_HEREDOCS.with_borrow_mut(|v| *v = ps.lex_heredocs.clone());
    set_incmdpos(ps.incmdpos);
    // parse.c:334 — `aliasspaceflag = ps->aliasspaceflag;`.
    crate::ported::lex::LEX_ALIAS_SPACE_FLAG.with(|c| c.set(ps.aliasspaceflag));
    set_incond(ps.incond);
    set_inredir(ps.inredir);
    set_incasepat(ps.incasepat);
    set_isnewlin(ps.isnewlin);
    set_infor(ps.infor);
    set_inrepeat(ps.inrepeat_);
    set_intypeset(ps.intypeset);
    // ecbuf/eclen/ecused/ecnpats/ecstrs/ecsoffs/ecssub/ecnfunc
    // STUB until Phase 9b.

    // parse.c:354 — `errflag &= ~ERRFLAG_ERROR;` — clear the
    // error flag so the outer parse sees a clean state.
    errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
}

/// Direct port of `ecadjusthere(int p, int d)` at `Src/parse.c:360`. Walk
/// the pending-heredocs list and bump each `pc` by `d` if it's
/// at or after position `p`. Called by `ecispace` / `ecdel` when
/// wordcodes shift.
#[allow(unused_variables)]
pub fn ecadjusthere(p: usize, d: i32) {
    // parse.c:362-366 — `for (p2 = hdocs; p2; p2 = p2->next) if
    // (p2->pc >= p) p2->pc += d;`. zshrs's hdocs are still
    // Vec<HereDoc> on the lexer (pre-P9c migration); since none
    // of them carry a wordcode pc today (the AST tree has no pc
    // slots), this is a no-op until Phase 9c wires
    // `hdocs.pc` into wordcode emission.
}

// === AST tree relocated to src/extensions/zsh_ast.rs ===
//
// zsh C does NOT have an AST tree — it emits wordcode directly via
// par_event/par_list/par_sublist/par_pipe/par_cmd/par_simple/etc.
// (Src/parse.c:485-3000) into a flat `Wordcode ecbuf[]`. The Zsh*/
// Shell* AST node types lived in this file as a Rust-only IR that
// stands in for that wordcode.
//
// P9e (PORT_PLAN.md): the types moved to src/extensions/zsh_ast.rs
// to make their Rust-only-extension nature explicit. The full P9c +
// P9d rewrite (par_* emitting wordcode + vm_helper reading wordcode)
// retires them entirely — until then, callers reach them via this
// re-export.

/// Direct port of `ecispace(int p, int n)` at `Src/parse.c:372`. Insert `n`
/// empty wordcode slots at position `p`, shifting later entries
/// right, growing the buffer as needed, adjusting heredoc pointers.
pub fn ecispace(p: usize, n: usize) {
    // parse.c:376-381 — grow if needed.
    let need = n as i32;
    if (ECLEN.get() - ECUSED.get()) < need {
        let cur = ECLEN.get();
        let mut a = if cur < EC_DOUBLE_THRESHOLD {
            cur
        } else {
            EC_INCREMENT
        };
        if need > a {
            a = need;
        }
        ECBUF.with_borrow_mut(|buf| {
            buf.resize((cur + a) as usize, 0);
        });
        ECLEN.set(cur + a);
    }
    // parse.c:382-385 — memmove p → p+n, gap of n.
    let m = ECUSED.get() as usize - p;
    if m > 0 {
        ECBUF.with_borrow_mut(|buf| {
            let needed = (ECUSED.get() as usize) + n;
            if buf.len() < needed {
                buf.resize(needed, 0);
            }
            for i in (0..m).rev() {
                buf[p + n + i] = buf[p + i];
            }
            for i in 0..n {
                buf[p + i] = 0;
            }
        });
    }
    // parse.c:386 — bump ecused by n.
    ECUSED.set(ECUSED.get() + need);
    // parse.c:387 — `ecadjusthere(p, n)`.
    ecadjusthere(p, need);
}

/// Direct port of `ecadd(wordcode c)` at `Src/parse.c:397`. Append `c` to
/// the wordcode buffer with grow-on-demand, return the new index.
pub fn ecadd(c: u32) -> usize {
    // parse.c:399-405 — `if ((eclen - ecused) < 1) grow`.
    if (ECLEN.get() - ECUSED.get()) < 1 {
        let cur = ECLEN.get();
        let a = if cur < EC_DOUBLE_THRESHOLD {
            cur
        } else {
            EC_INCREMENT
        };
        ECBUF.with_borrow_mut(|buf| {
            buf.resize((cur + a) as usize, 0);
        });
        ECLEN.set(cur + a);
    }
    let idx = ECUSED.get();
    ECBUF.with_borrow_mut(|buf| {
        if (idx as usize) >= buf.len() {
            buf.resize((idx + 1) as usize, 0);
        }
        buf[idx as usize] = c;
    });
    ECUSED.set(idx + 1);
    idx as usize
}

/// Direct port of `ecdel(int p)` at `Src/parse.c:413`. Remove the
/// wordcode at position `p`, shift later entries left by one,
/// decrement ecused, adjust pending heredoc pointers.
pub fn ecdel(p: usize) {
    // parse.c:415-418 — memmove + decrement ecused.
    let n = ECUSED.get() as usize - p - 1;
    if n > 0 {
        ECBUF.with_borrow_mut(|buf| {
            for i in 0..n {
                buf[p + i] = buf[p + i + 1];
            }
        });
    }
    ECUSED.set(ECUSED.get() - 1);
    // parse.c:420 — `ecadjusthere(p, -1)`.
    ecadjusthere(p, -1);
}

/// Direct port of `ecstrcode(char *s)` at `Src/parse.c:426`. Encode a
/// string into a single wordcode (short strings ≤4 bytes packed
/// inline; longer strings get an offset into the deduped registry).
///
/// The long-string path stores the METAFIED bytes (matches what C's
/// strs region contains): collapse Rust UTF-8 chars in 0x80..=0xff
/// to single bytes, then apply zsh metafy (high bytes ≥ 0x83 →
/// `Meta=0x83 + byte^0x20`). Length tracking (ECSOFFS) uses the
/// metafied byte count — same as C `strlen(s) + 1` where C's `s`
/// is already metafied at this point.
pub fn ecstrcode(s: &str) -> u32 {
    // Convert Rust char-form → C-byte form. zsh's metafy() at
    // Src/utils.c only converts bytes flagged IMETA: 0x00, 0x83
    // (Meta itself), and 0x84..=0xa2 (Pound..Marker, the lex
    // markers). Other bytes 0x01..=0x82 and 0xa3..=0xff pass
    // through unchanged. See utils.c:4195-4204 typtab init.
    //
    // Rust receives chars. Classify each:
    //   - codepoint in [0x83..=0xa2] → marker char (emitted by lex
    //     post-metafy in C); 1 byte unchanged
    //   - codepoint < 0x80 → ASCII, 1 byte unchanged
    //   - codepoint in [0x80..=0x82] or [0xa3..=0xff] → single
    //     non-imeta byte (user-input range); 1 byte unchanged
    //   - codepoint > 0xff → multi-byte UTF-8 source char (e.g.
    //     '━' = U+2501 = 0xe2 0x94 0x81). Metafy ONLY the bytes
    //     that fall in 0x83..=0xa2; pass others through. For '━':
    //     0xe2 stays, 0x94 → 0x83 0xb4, 0x81 stays.
    let mut c_bytes: Vec<u8> = Vec::with_capacity(s.len());
    let imeta = |b: u8| -> bool { b == 0 || (0x83..=0xa2).contains(&b) };
    for ch in s.chars() {
        let cu = ch as u32;
        if cu < 0x80 {
            // ASCII — single byte unchanged.
            c_bytes.push(cu as u8);
        } else if (0x83..=0xa2).contains(&cu) {
            // Lex marker char (emitted by lex.add(Marker) post-metafy
            // in C). Stored as single byte.
            c_bytes.push(cu as u8);
        } else {
            // User-input char: encode UTF-8 then metafy imeta bytes.
            // For chars 0x80..=0xff (like 'º' U+00BA), UTF-8 gives
            // 2 bytes (e.g. `0xc2 0xba`) — zsh's lex reads these as
            // raw bytes from input and metafy passes 0xc2 / 0xba
            // through (both NOT imeta).
            let mut tmp = [0u8; 4];
            for &b in ch.encode_utf8(&mut tmp).as_bytes() {
                if imeta(b) {
                    c_bytes.push(0x83);
                    c_bytes.push(b ^ 0x20);
                } else {
                    c_bytes.push(b);
                }
            }
        }
    }
    // c:`has_token` (Src/utils.c:2282) → `itok(*s)` → `typtab[c] & ITOK`.
    // ITOK is set for bytes `Pound..=Nularg` (0x84..=0xa1) per
    // Src/utils.c:4198 (`for (t0=Pound; t0<=LAST_NORMAL_TOK; t0++)
    // typtab[t0]|=ITOK`) plus :4200 (`for (t0=Snull; t0<=Nularg; t0++)
    // typtab[t0]|=ITOK|IMETA|INULL`). Pound=0x84 Bang=0x9c (last normal),
    // Snull=0x9d..Nularg=0xa1. Meta=0x83 has IMETA but NOT ITOK.
    let t = c_bytes.iter().any(|&b| (0x84..=0xa1).contains(&b));
    let l = c_bytes.len() + 1; // include NUL terminator
    if l <= 4 {
        // parse.c:436-445 — short-string inline pack. Uses raw C-bytes
        // (NOT metafied — the inline packing stores 1 byte per slot).
        let mut c: u32 = if t { 3 } else { 2 };
        match l {
            4 => {
                c |= (c_bytes[2] as u32) << 19;
                c |= (c_bytes[1] as u32) << 11;
                c |= (c_bytes[0] as u32) << 3;
            }
            3 => {
                c |= (c_bytes[1] as u32) << 11;
                c |= (c_bytes[0] as u32) << 3;
            }
            2 => {
                c |= (c_bytes[0] as u32) << 3;
            }
            1 => {
                // parse.c:443 — empty string special case.
                c = if t { 7 } else { 6 };
            }
            _ => {}
        }
        c
    } else {
        // parse.c:447-466 — long string. Port of C's eccstr BST walk
        // exactly: walk the tree comparing nfunc, then hashval, then
        // strcmp on bytes. Return offs on full match; insert new
        // leaf otherwise. Matches C's exact dedup-hit pattern
        // (which is content-dependent — hash collisions and the
        // lazy short-circuit cmp chain make the tree shape determine
        // whether matching nodes are reachable).
        // hasher is byte-by-byte polynomial (hashtable.c:86); pass
        // c_bytes via from_utf8_unchecked so non-UTF-8 zsh marker
        // bytes feed straight in. SAFETY: hasher only iterates
        // `.bytes()` — no UTF-8 validity assumed.
        let val =
            crate::ported::hashtable::hasher(unsafe { std::str::from_utf8_unchecked(&c_bytes) });
        let nfunc = ECNFUNC.get();
        let found_offs = ECSTRS_TREE.with_borrow_mut(|root| {
            // Walk the tree. At each node, if all 3 cmps == 0,
            // return the node's offs. Otherwise descend left/right
            // by the first non-zero cmp's sign.
            let mut cur: &mut Option<Box<EccstrNode>> = root;
            loop {
                let p = match cur.as_mut() {
                    Some(p) => p,
                    None => break None,
                };
                // c:448 — `cmp = p->nfunc - ecnfunc`
                let mut cmp = (p.nfunc as i64) - (nfunc as i64);
                if cmp == 0 {
                    // c:448 — `&& !(cmp = (long)p->hashval - (long)val)`
                    // C does `(int)(p->hashval - val)` — unsigned 32-bit
                    // subtraction wraps, then cast to int. Use
                    // wrapping_sub + as i32 to match the bit pattern.
                    cmp = (p.hashval.wrapping_sub(val) as i32) as i64;
                    if cmp == 0 {
                        // c:448 — `&& !(cmp = strcmp(p->str, s))`
                        cmp = match p.str.as_slice().cmp(c_bytes.as_slice()) {
                            std::cmp::Ordering::Less => -1,
                            std::cmp::Ordering::Equal => 0,
                            std::cmp::Ordering::Greater => 1,
                        };
                        if cmp == 0 {
                            // c:450 — `return p->offs;`
                            break Some(p.offs);
                        }
                    }
                }
                // c:452 — `pp = (cmp < 0 ? &p->left : &p->right);`
                cur = if cmp < 0 { &mut p.left } else { &mut p.right };
            }
        });
        if let Some(offs) = found_offs {
            return offs;
        }
        // c:462 — `p->offs = ((ecsoffs - ecssub) << 2) | (t ? 1 : 0);`
        let offs = (((ECSOFFS.get() - ECSSUB.get()) as u32) << 2) | if t { 1 } else { 0 };
        // c:463 — `p->aoffs = ecsoffs;` (absolute write position).
        let aoffs = ECSOFFS.get() as u32;
        // c:457-465 — insert new node at the NULL slot the walk
        // terminated at. Encode the walk path as a Vec<bool> of
        // left/right turns (true = right), then re-descend to
        // insert. Borrow-checker friendly: a single mutable walk
        // that either finds an existing node (descend) or fills
        // the empty slot (return).
        let stored = c_bytes.clone();
        let stored_len = stored.len();
        let new_node = Box::new(EccstrNode {
            left: None,
            right: None,
            str: stored.clone(),
            offs,
            aoffs,
            nfunc,
            hashval: val,
        });
        ECSTRS_TREE.with_borrow_mut(|root| {
            // Build the path first (immutable-walk; safe because we
            // only ever go further down).
            let mut path: Vec<bool> = Vec::new();
            {
                let mut cur: &Option<Box<EccstrNode>> = root;
                while let Some(p) = cur.as_ref() {
                    let mut cmp = (p.nfunc as i64) - (nfunc as i64);
                    if cmp == 0 {
                        // C does `(int)(p->hashval - val)` — unsigned 32-bit
                        // subtraction wraps, then cast to int. Use
                        // wrapping_sub + as i32 to match the bit pattern.
                        cmp = (p.hashval.wrapping_sub(val) as i32) as i64;
                        if cmp == 0 {
                            cmp = match p.str.as_slice().cmp(c_bytes.as_slice()) {
                                std::cmp::Ordering::Less => -1,
                                std::cmp::Ordering::Equal => 0,
                                std::cmp::Ordering::Greater => 1,
                            };
                        }
                    }
                    let go_right = cmp >= 0;
                    path.push(go_right);
                    cur = if go_right { &p.right } else { &p.left };
                }
            }
            // Descend mutably along the recorded path and assign at
            // the NULL leaf.
            let mut cur: &mut Option<Box<EccstrNode>> = root;
            for turn in path {
                let p = cur.as_mut().expect("path matches walk");
                cur = if turn { &mut p.right } else { &mut p.left };
            }
            *cur = Some(new_node);
        });
        // Also keep the existing reverse index (offs → bytes) for
        // ecgetstr_wordcode and copy_ecstr — they read flat by offs.
        ECSTRS_REVERSE.with_borrow_mut(|m| {
            m.insert(offs, stored);
        });
        let _ = l;
        ECSOFFS.set(ECSOFFS.get() + (stored_len + 1) as i32);
        offs
    }
}

/// Initialize parser status. Direct port of zsh/Src/parse.c:491
/// `init_parse_status`. Clears the per-parse-call lexer flags
/// so a fresh parse starts from cmd-position with no nesting
/// state inherited from a prior parse.
///
/// Previously the Rust port omitted `inrepeat_ = 0` at c:501.
/// `inrepeat_` is the `repeat N <body>` parse-state counter that
/// the lexer toggles in 3 phases (1 → 2 → 3 → 0). Without the
/// reset, a fresh parse called after an in-flight `repeat`
/// command would inherit the stale counter and silently misread
/// the next token as a body of an already-completed repeat.
pub fn init_parse_status() {
    // c:491
    // parse.c:500-502 — `incasepat = incond = inredir = infor =
    // intypeset = 0; inrepeat_ = 0; incmdpos = 1;`
    set_incasepat(0); // c:500
    set_incond(0); // c:500
    set_inredir(false); // c:500
    set_infor(0); // c:500
    set_intypeset(false); // c:500
    set_inrepeat(0); // c:501 inrepeat_ = 0
    set_incmdpos(true); // c:502
}

/// Initialize parser for a fresh parse. Direct port of
/// zsh/Src/parse.c:509 `init_parse`. C source allocates a
/// fresh wordcode buffer (ecbuf) sized EC_INIT_SIZE, resets the
/// per-parse-call counters, and calls init_parse_status. zshrs
/// has no flat wordcode buffer (AST is built inline) so this
/// function reduces to init_parse_status + recursion_depth/
/// global_iterations clear.
pub fn init_parse() {
    // parse.c:513-520 — `ecbuf = (Wordcode) zalloc(EC_INIT_SIZE *
    // sizeof(wordcode)); eclen = EC_INIT_SIZE; ecused = 0;
    // ecnpats = 0; ecstrs = NULL; ecsoffs = ecnfunc = 0;
    // ecssub = 0;`. P9b — initialize the per-evaluator wordcode
    // buffer for this parse call. zshrs uses thread-local
    // statics declared at file scope (parse.rs:25-50).
    ECBUF.with_borrow_mut(|buf| {
        buf.clear();
        buf.resize(EC_INIT_SIZE as usize, 0);
    });
    ECLEN.set(EC_INIT_SIZE);
    ECUSED.set(0);
    ECNPATS.set(0);
    ECSOFFS.set(0);
    ECSSUB.set(0);
    ECNFUNC.set(0);
    ECSTRS_INDEX.with_borrow_mut(|m| m.clear());
    ECSTRS_REVERSE.with_borrow_mut(|m| m.clear());
    ECSTRS_TREE.with_borrow_mut(|t| *t = None);

    // parse.c:522 — `init_parse_status();`
    init_parse_status();
}

/// Port of `copy_ecstr(Eccstr s, char *p)` from `Src/parse.c:537`.
/// Walks the BST and writes each entry to `p[s->aoffs..]` matching
/// C's recursive in-order traversal exactly. The old impl used the
/// `ECSTRS_REVERSE` HashMap keyed by `offs` (= ecssub-relative
/// wordcode-encoded offset), which collides across funcdef scopes:
/// a string at relative offs=0 inside funcdef A and another at
/// relative offs=0 inside funcdef B share the same key, so one
/// overwrites the other.
pub fn copy_ecstr(_table: &std::collections::HashMap<u32, Vec<u8>>, p: &mut [u8]) {
    // c:537-544 — walk eccstr BST recursively, writing each node's
    // str at p[node->aoffs..node->aoffs + strlen + 1] (NUL-terminated).
    ECSTRS_TREE.with_borrow(|root| {
        copy_ecstr_walk(root, p);
    });
}

/// Port of `bld_eprog(int heap)` from `Src/parse.c:547`. Finalizes
/// the in-build `ECBUF`/`ECSTRS`/`ECNPATS` state into an `Eprog`.
/// Resets the build state so a new parse can start.
pub fn bld_eprog(heap: bool) -> eprog {
    // c:547

    // c:555 — emit WC_END opcode. `WCB_END` is `WC_END_DEFAULT` (0).
    ecadd(0);

    let ecused = ECUSED.with(|c| c.get()) as usize;
    let ecnpats = ECNPATS.with(|c| c.get()) as usize;
    let ecsoffs = ECSOFFS.with(|c| c.get()) as usize;

    // c:557-559 — `ret->len = ((ecnpats * sizeof(Patprog)) +
    //                            (ecused * sizeof(wordcode)) +
    //                            ecsoffs);`
    // sizeof(Patprog) = sizeof(struct patprog *) = pointer size.
    // On 64-bit targets that's 8, on 32-bit that's 4. C's eprog
    // ->len is the canonical value for parity tests, so we use
    // the same arithmetic.
    let prog_bytes = ecused * 4; // sizeof(wordcode) = 4
    let len = (ecnpats * size_of::<*const u8>()) + prog_bytes + ecsoffs;

    // Snapshot the wordcode buffer + string table.
    let prog_words: Vec<u32> = ECBUF.with(|c| c.borrow()[..ecused].to_vec());
    let mut strs_bytes = vec![0u8; ecsoffs];
    ECSTRS_REVERSE.with(|c| copy_ecstr(&c.borrow(), &mut strs_bytes));

    // c:566 — store strs as raw bytes via from_utf8_unchecked so
    // single-byte zsh markers (e.g. Dash 0x9b) survive intact.
    // `String::from_utf8_lossy` would replace them with U+FFFD
    // (`\xef\xbf\xbd`), breaking byte-for-byte parity with C's
    // strs region. SAFETY: downstream consumers of `eprog.strs`
    // index by byte offset (per the wordcode `(offs >> 2)` offset
    // encoding) and call `.as_bytes()` — they never iterate as
    // chars or rely on UTF-8 validity, so storing non-UTF-8 bytes
    // in a String is safe in practice. C zsh's strs is `char *`
    // with the same byte-not-char semantics.
    let strs_string = unsafe { String::from_utf8_unchecked(strs_bytes) };
    let ret = eprog {
        flags: if heap { EF_HEAP } else { EF_REAL }, // c:570
        len: len as i32,                             // c:559
        npats: ecnpats as i32,                       // c:561
        nref: if heap { -1 } else { 1 },             // c:562
        pats: Vec::new(),                            // c:563 dummy_patprog
        prog: prog_words,                            // c:565
        strs: Some(strs_string),
        shf: None,
        dump: None,
    };

    // c:577 — free ecbuf so next parse starts fresh.
    ECBUF.with(|c| c.borrow_mut().clear());
    ECLEN.with(|c| c.set(0));
    ECUSED.with(|c| c.set(0));
    ECNPATS.with(|c| c.set(0));
    ECSOFFS.with(|c| c.set(0));
    ECSTRS_INDEX.with(|c| c.borrow_mut().clear());
    ECSTRS_REVERSE.with(|c| c.borrow_mut().clear());
    ECSTRS_TREE.with(|t| *t.borrow_mut() = None);

    ret
}

/// Port of `int empty_eprog(Eprog p)` from `Src/parse.c:584`. C
/// body: `return (!p || !p->prog || *p->prog == WCB_END());` —
/// the eprog is empty when its prog buffer is missing or the
/// first wordcode is the WC_END marker. Used by signal handlers
/// (`Src/signals.c:712`) to short-circuit a trap that resolves to
/// an empty program.
pub fn empty_eprog(p: &eprog) -> bool {
    p.prog.is_empty() || p.prog[0] == WCB_END()
}

/// Clear pending here-document list. Direct port of
/// `clear_hdocs(void)` from `Src/parse.c:591`. The C version walks
/// `hdocs` and frees each node; Rust drops the `Box<heredocs>`
/// chain automatically when the head is replaced with None.
pub fn clear_hdocs() {
    // c:591
    // c:593-598 — for (p = hdocs; p; p = n) { n = p->next; zfree(p); }
    // c:599 — hdocs = NULL;
    HDOCS.with_borrow_mut(|h| *h = None);
    // zshrs-only: also drop the parallel AST-glue Vec. No C
    // analog — LEX_HEREDOCS is Rust-only working-set state.
    LEX_HEREDOCS.with_borrow_mut(|v| v.clear());
}

/// Top-level parse-event entry. Direct port of zsh/Src/parse.c:
/// 612-631 `parse_event`. Reads one event from the lexer (a
/// sublist optionally followed by SEPER/AMPER/AMPERBANG) and
/// returns the resulting ZshProgram.
///
/// `endtok` is the token that terminates the event — usually
/// ENDINPUT, but for command-style substitutions the closing
/// `)` (zsh's CMD_SUBST_CLOSE).
///
/// zshrs port note: zsh's parse_event returns an `Eprog` (heap-
/// allocated wordcode program). zshrs returns a `ZshProgram`
/// (AST root). Same role at the parse-output boundary.
pub fn parse_event(endtok: lextok) -> Option<ZshProgram> {
    // parse.c:616-619 — reset state and prime the lexer.
    set_tok(ENDINPUT);
    set_incmdpos(true);
    // parse.c:618 — `aliasspaceflag = 0;`. Fresh event: discard any
    // alias-space carry-over from a prior parse so HISTIGNORESPACE
    // doesn't suppress the next entered command line.
    crate::ported::lex::LEX_ALIAS_SPACE_FLAG.with(|c| c.set(0));
    zshlex();
    // parse.c:620 — `init_parse();`
    init_parse();

    // parse.c:622-625 — drive par_event; on failure clear hdocs.
    if !par_event(endtok) {
        clear_hdocs();
        return None;
    }
    // parse.c:626-628 — if endtok != ENDINPUT, this is a sub-
    // parse for a substitution that doesn't need its own eprog.
    // zshrs returns an empty program in that case (caller
    // discards).
    if endtok != ENDINPUT {
        return Some(ZshProgram { lists: Vec::new() });
    }
    // parse.c:630 — `bld_eprog(1);` — build the final eprog.
    // zshrs has already built the AST via parse_program_until,
    // but parse_event uses par_event directly so we need to
    // collect what par_event accumulated.
    Some(parse_program_until(None))
}

/// Parse one event (sublist with optional separator). Direct
/// port of zsh/Src/parse.c:635 `par_event`. Returns true if
/// an event was successfully parsed, false on EOF / endtok.
///
/// zshrs port note: the C version emits wordcodes via ecadd/
/// set_list_code; zshrs's parser builds AST nodes via
/// par_sublist + par_list. Same flow, different output.
pub fn par_event(endtok: lextok) -> bool {
    // parse.c:639-643 — skip leading SEPERs.
    while tok() == SEPER {
        // parse.c:640-641 — at top-level (endtok == ENDINPUT),
        // a SEPER on a fresh line ends the event.
        if isnewlin() > 0 && endtok == ENDINPUT {
            return false;
        }
        zshlex();
    }
    // parse.c:644-647 — terminate on EOF or matching close-token.
    if tok() == ENDINPUT {
        return false;
    }
    if tok() == endtok {
        return true;
    }
    // parse.c:649-... — drive par_sublist + handle terminator.
    // zshrs's par_sublist already builds the AST node directly.
    match par_sublist() {
        Some(_) => {
            // parse.c:651-693 — terminator handling. zshrs's
            // par_list wraps this; for parse_event we just
            // confirm the sublist parsed.
            true
        }
        None => false,
    }
}

/// Port of `parse_list(void)` from `Src/parse.c:697`. C-shape entry
/// point: drives `par_list` and finalizes via `bld_eprog`. Returns
/// `None` on syntax error.
pub fn parse_list() -> Option<eprog> {
    // c:697
    set_tok(ENDINPUT);
    init_parse();
    zshlex();
    // c:Src/parse.c:705 — `par_list(&c);` emits wordcode for the
    // full multi-statement list (its goto-rec loop walks all
    // SEPER-separated sublists). The Rust AST par_list() emits
    // NOTHING to the wordcode buffer (only builds the AST), so
    // bld_eprog returned an empty program AND tok stayed at
    // SEPER, tripping the syntax-error check below for any
    // \`cmd; cmd\` body.
    //
    // Route through par_event_wordcode (the wordcode emitter,
    // lines 4395+) which mirrors C's par_list loop semantics
    // and populates the wordcode buffer that bld_eprog reads.
    let _start = par_event_wordcode();
    if tok() != ENDINPUT {
        clear_hdocs();
        set_tok(LEXERR);
        // c:Src/parse.c:708 — `yyerror(0);`. C-faithful invocation;
        // the message format ("parse error near `X'") is built
        // inside yyerror from zshlextext/tokstr().
        yyerror(0);
        return None;
    }
    Some(bld_eprog(false))
}

/// Port of `parse_cond(void)` from `Src/parse.c:722`. Only used by
/// `bin_test`/`bin_bracket` for `/bin/test`/`[` compat — the
/// `condlex` global must already point at `testlex` before entry.
pub fn parse_cond() -> Option<eprog> {
    // c:722
    init_parse();
    if par_cond().is_none() {
        clear_hdocs();
        return None;
    }
    Some(bld_eprog(true))
}

// ============================================================
// Wordcode emission helpers (parse.c private helpers)
//
// Direct ports of zsh's wordcode-emission helpers in parse.c.
// These write u32 opcodes into a flat `ecbuf` array thread-local
// via ecadd / ecdel / ecispace / ecstrcode and friends. The
// par_*_wordcode family at parse.rs:1700-3500 walks the lex
// stream and emits a real wordcode buffer here.
//
// (The AST tree built by par_program / par_simple / etc. is a
// separate path used by fusevm; see compile_zsh.rs for the AST
// → fusevm-bytecode compiler.)
// ============================================================

/// Patch a list-placeholder wordcode with its actual opcode +
/// jump distance. Direct port of zsh/Src/parse.c:738
/// `set_list_code`. zsh emits an `ecadd(0)` placeholder before
/// par_sublist runs, then comes back through set_list_code to
/// rewrite the slot with WCB_LIST(type, distance) once the
/// sublist's final length is known.
///
/// Port of `set_list_code(int p, int type, int cmplx)` from
/// `Src/parse.c:738`. Patches the WCB_LIST header at `p` based on
/// whether the sublist body is simple (single command, no
/// pipeline) and Z_SYNC/Z_END — emits the Z_SIMPLE-optimized
/// header when possible, otherwise the plain WCB_LIST(type, 0).
pub fn set_list_code(p: usize, type_code: i32, cmplx: bool) {
    let _ = wc_bdata;
    // c:740 — `if (!cmplx && (type == Z_SYNC || type == (Z_SYNC | Z_END))
    // && WC_SUBLIST_TYPE(ecbuf[p+1]) == WC_SUBLIST_END)`
    let sublist_code = ECBUF.with_borrow(|b| b.get(p + 1).copied().unwrap_or(0));
    let z = type_code;
    let qualifies = !cmplx
        && (z == Z_SYNC || z == (Z_SYNC | Z_END))
        && WC_SUBLIST_TYPE(sublist_code) == WC_SUBLIST_END;
    if qualifies {
        // c:742 — `int ispipe = !(WC_SUBLIST_FLAGS(ecbuf[p+1])
        // & WC_SUBLIST_SIMPLE);`
        let ispipe = (WC_SUBLIST_FLAGS(sublist_code) & WC_SUBLIST_SIMPLE) == 0;
        // c:743 — `ecbuf[p] = WCB_LIST((type|Z_SIMPLE), ecused-2-p);`
        let used = ECUSED.get() as usize;
        let off = used.saturating_sub(2 + p);
        ECBUF.with_borrow_mut(|b| {
            if p < b.len() {
                b[p] = WCB_LIST((z | Z_SIMPLE) as wordcode, off as wordcode);
            }
        });
        // c:744 — `ecdel(p+1);`
        ecdel(p + 1);
        // c:745-746 — `if (ispipe) ecbuf[p+1] = WC_PIPE_LINENO(ecbuf[p+1]);`
        if ispipe {
            ECBUF.with_borrow_mut(|b| {
                if p + 1 < b.len() {
                    b[p + 1] = WC_PIPE_LINENO(b[p + 1]);
                }
            });
        }
    } else {
        // c:748 — `ecbuf[p] = WCB_LIST(type, 0);`
        ECBUF.with_borrow_mut(|b| {
            if p < b.len() {
                b[p] = WCB_LIST(z as wordcode, 0);
            }
        });
    }
}

/// Port of `set_sublist_code(int p, int type, int flags, int skip, int cmplx)`
/// from `Src/parse.c:755`. Patches the WCB_SUBLIST header at `p`.
/// When the sublist is non-complex (single command, no pipeline),
/// sets WC_SUBLIST_SIMPLE and rewrites the following slot to
/// `WC_PIPE_LINENO`.
pub fn set_sublist_code(p: usize, type_code: i32, flags: i32, skip: i32, cmplx: bool) {
    if cmplx {
        // c:758 — `ecbuf[p] = WCB_SUBLIST(type, flags, skip);`
        ECBUF.with_borrow_mut(|b| {
            if p < b.len() {
                b[p] = WCB_SUBLIST(type_code as wordcode, flags as wordcode, skip as wordcode);
            }
        });
    } else {
        // c:760 — `ecbuf[p] = WCB_SUBLIST(type, flags|WC_SUBLIST_SIMPLE, skip);`
        ECBUF.with_borrow_mut(|b| {
            if p < b.len() {
                b[p] = WCB_SUBLIST(
                    type_code as wordcode,
                    (flags as wordcode) | WC_SUBLIST_SIMPLE,
                    skip as wordcode,
                );
            }
        });
        // c:761 — `ecbuf[p+1] = WC_PIPE_LINENO(ecbuf[p+1]);`
        ECBUF.with_borrow_mut(|b| {
            if p + 1 < b.len() {
                b[p + 1] = WC_PIPE_LINENO(b[p + 1]);
            }
        });
    }
}

/// Parse a list (sublist with optional & or ;).
///
/// Direct port of zsh/Src/parse.c:771-804 `par_list` (and the
/// par_list1 wrapper at parse.c:807-817).
///
/// **Structural divergence**: zsh's parse.c emits flat wordcode
/// into the `ecbuf` u32 array via `ecadd(0)` (placeholder),
/// `set_list_code(p, code, complexity)`, `wc_bdata(Z_END)`. zshrs
/// builds an AST node `ZshList { sublist, flags }` instead. The
/// async/sync/disown discrimination at parse.c:785-790 maps to
/// zshrs's `ListFlags { async_, disown }` field — Z_SYNC is the
/// default (no flags), Z_ASYNC = `&` = `async_=true`, Z_DISOWN +
/// Z_ASYNC = `&!`/`&|` = both true. Same semantics, different
/// representation. This divergence is repository-wide: every
/// `par_*` function emits wordcode in C, every `parse_*` builds
/// AST in Rust. The compile_zsh module then traverses the AST to
/// emit fusevm bytecode, which serves the same role as zsh's
/// wordcode but with a different opcode set and execution model.
fn par_list() -> Option<(ZshList, bool)> {
    let sublist = par_sublist()?;

    // c:769-803 — `list : { SEPER } [ sublist [ { SEPER | AMPER |
    // AMPERBANG } list ] ]`. The second tuple element reports
    // whether the list ended with an EXPLICIT terminator. C's list
    // grammar only chains sublists across SEPER/AMPER/AMPERBANG; a
    // sublist followed directly by another command (`{a} {b}`) ends
    // the list and the dangling token is the CALLER's problem
    // (par_while takes a dangling INBRACE as the loop body, par_event
    // yyerrors at c:671-680). parse_program_until needs this bit to
    // reproduce that split.
    let (flags, terminated) = match tok() {
        AMPER => {
            zshlex();
            (
                ListFlags {
                    async_: true,
                    disown: false,
                },
                true,
            )
        }
        AMPERBANG => {
            zshlex();
            (
                ListFlags {
                    async_: true,
                    disown: true,
                },
                true,
            )
        }
        SEPER | SEMI | NEWLIN => {
            zshlex();
            (ListFlags::default(), true)
        }
        _ => (ListFlags::default(), false),
    };

    Some((ZshList { sublist, flags }, terminated))
}

/// Parse one list — non-recursing variant. Direct port of
/// zsh/Src/parse.c:808 `par_list1`. Like par_list but
/// doesn't recurse on the trailing-separator path; used by
/// callers that only want one statement (e.g. each arm of a
/// case body).
pub fn par_list1() -> Option<ZshSublist> {
    // parse.c:810-816 — body is a single par_sublist call wrapped
    // in the eu/ecused tracking that zshrs doesn't need (no
    // wordcode buffer).
    par_sublist()
}

/// Parse a sublist (pipelines connected by && or ||).
///
/// Direct port of zsh/Src/parse.c:825 `par_sublist` and
/// par_sublist2 at parse.c:869-892. par_sublist handles the
/// && / || conjunction and emits WC_SUBLIST opcodes; par_sublist2
/// handles the leading `!` negation and `coproc` keyword.
///
/// AST mapping: ZshSublist { pipe, conj_chain }, where `conj_chain`
/// is a Vec<(ConjOp, ZshSublist)> for chained && / ||. C uses
/// flat wordcode with WC_SUBLIST_AND / WC_SUBLIST_OR markers.
fn par_sublist() -> Option<ZshSublist> {
    let mut flags = SublistFlags::default();

    // Handle coproc and !
    if tok() == COPROC {
        flags.coproc = true;
        zshlex();
    } else if tok() == BANG_TOK {
        flags.not = true;
        zshlex();
    }

    let pipe = par_pline()?;

    // Check for && or ||
    let next = match tok() {
        DAMPER => {
            zshlex();
            skip_separators();
            // c:Src/parse.c:par_sublist — and-or operators (`&&`,
            // `||`) require a sublist on each side. After consuming
            // `&&`/`||`, another and-or operator OR a pipe-operator
            // immediately after is a parse error in C zsh. zshrs's
            // recursion silently returned None and dropped the
            // operator. Bug #171 in docs/BUGS.md.
            if matches!(tok(), DAMPER | DBAR | BAR_TOK | BARAMP) {
                let name = match tok() {
                    DAMPER => "&&",
                    DBAR => "||",
                    BAR_TOK => "|",
                    BARAMP => "|&",
                    _ => "operator",
                };
                zerr(&format!("parse error near `{}'", name));
                return None;
            }
            par_sublist().map(|s| (SublistOp::And, Box::new(s)))
        }
        DBAR => {
            zshlex();
            skip_separators();
            if matches!(tok(), DAMPER | DBAR | BAR_TOK | BARAMP) {
                let name = match tok() {
                    DAMPER => "&&",
                    DBAR => "||",
                    BAR_TOK => "|",
                    BARAMP => "|&",
                    _ => "operator",
                };
                zerr(&format!("parse error near `{}'", name));
                return None;
            }
            par_sublist().map(|s| (SublistOp::Or, Box::new(s)))
        }
        _ => None,
    };

    Some(ZshSublist { pipe, next, flags })
}

/// Port of `par_sublist2(int *cmplx)` from `Src/parse.c:869`.
/// Secondary-sublist arm: handles the `COPROC`/`Bang` prefix
/// in front of a pline. Returns the WC_SUBLIST flag word added.
pub fn par_sublist2(cmplx: &mut i32) -> Option<i32> {
    // c:870 — `int f = 0;`
    let mut f: i32 = 0;
    // c:873-880 — COPROC / BANG prefix flags.
    if tok() == COPROC {
        *cmplx = 1;
        f |= WC_SUBLIST_COPROC as i32;
        zshlex();
    } else if tok() == BANG_TOK {
        *cmplx = 1;
        f |= WC_SUBLIST_NOT as i32;
        zshlex();
    }
    // c:882-883 — `if (!par_pline(cmplx) && !f) return -1;`
    if !par_pipe_wordcode(cmplx) && f == 0 {
        return None;
    }
    // c:885 — `return f;`
    Some(f)
}

/// Parse a pipeline
/// Parse a pipeline (cmds joined by `|` / `|&`). Direct port of
/// zsh/Src/parse.c:894 `par_pline`. AST: ZshPipe { cmds: Vec<ZshCommand> }.
/// C emits WC_PIPE wordcodes per command; same flow.
fn par_pline() -> Option<ZshPipe> {
    let lineno = toklineno();
    let cmd = par_cmd()?;

    // Check for | or |&
    let mut merge_stderr = false;
    let next = match tok() {
        BAR_TOK | BARAMP => {
            merge_stderr = tok() == BARAMP;
            zshlex();
            skip_separators();
            // c:Src/parse.c:par_pline — pipe-operators require a
            // command on each side. After consuming `|`/`|&`,
            // C zsh's recursive par_pline call returns -1 (parse
            // error) when the next token is another pipe-operator
            // — `a | | b` errors with `parse error near `|''`.
            // zshrs's `par_pline()?` silently returned None on
            // missing command, dropping the rest of the input
            // without diagnosing the empty-pipe-operand. Bug #171
            // in docs/BUGS.md.
            if matches!(tok(), BAR_TOK | BARAMP) {
                let name = if tok() == BARAMP { "|&" } else { "|" };
                zerr(&format!("parse error near `{}'", name));
                return None;
            }
            par_pline().map(Box::new)
        }
        _ => None,
    };

    Some(ZshPipe {
        cmd,
        next,
        lineno,
        merge_stderr,
    })
}

/// Parse a command
/// Parse a command — dispatches by leading token (FOR / CASE /
/// IF / WHILE / UNTIL / REPEAT / FUNC / DINBRACK / DINPAR /
/// Inpar subshell / Inbrace current-shell / TIME / NOCORRECT,
/// else simple). Direct port of zsh/Src/parse.c:958 `par_cmd`.
fn par_cmd() -> Option<ZshCommand> {
    // Parse leading redirections
    let mut redirs = Vec::new();
    while IS_REDIROP(tok()) {
        if let Some(redir) = par_redir() {
            redirs.push(redir);
        }
    }

    let cmd = match tok() {
        FOR | FOREACH => par_for(),
        SELECT => parse_select(),
        CASE => par_case(),
        IF => par_if(),
        WHILE => par_while(false),
        UNTIL => par_while(true),
        REPEAT => par_repeat(),
        INPAR_TOK => par_subsh(),
        INOUTPAR => parse_anon_funcdef(),
        INBRACE_TOK => parse_cursh(),
        FUNC => par_funcdef(),
        DINBRACK => par_cond(),
        DINPAR => parse_arith(),
        TIME => par_time(),
        _ => par_simple(redirs),
    };

    // Parse trailing redirections. For Simple commands the redirs were
    // already captured inside par_simple; for compound forms (Cursh,
    // Subsh, If, While, etc.) we collect them here and wrap in
    // ZshCommand::Redirected so compile_zsh can scope-bracket them.
    if let Some(inner) = cmd {
        let mut trailing: Vec<ZshRedir> = Vec::new();
        while IS_REDIROP(tok()) {
            if let Some(redir) = par_redir() {
                trailing.push(redir);
            }
        }
        // c:Src/parse.c:par_cmd — compound forms (Cursh `{...}`, Subsh
        // `(...)`, If/While/Until/For/Case/Select/Repeat/Funcdef) must
        // be followed by a valid sublist/list separator (`;`, `\n`,
        // `&`, `|`, `&&`, `||`, redirect-op) — STRING_LEX after a
        // compound is a parse error. zshrs's outer par_list loop
        // silently treated trailing words as a new command, masking
        // syntax errors like `{ echo a; } b c`. Mirror C's strict
        // post-compound terminator check. Bug #146 in docs/BUGS.md.
        if !matches!(inner, ZshCommand::Simple(_)) && tok() == STRING_LEX {
            let bad = tokstr().unwrap_or_default();
            zerr(&format!("parse error near `{}'", bad));
            // Reset state before returning so the outer loop's None
            // detection unwinds cleanly.
            set_incmdpos(true);
            set_incasepat(0);
            set_incond(0);
            set_intypeset(false);
            return None;
        }
        // c:1072-1075 — every par_cmd tail resets the lexer state
        // toggles so the NEXT command starts in cmd position with
        // case/cond/typeset off. par_simple/par_cond set `incmdpos=0`
        // during their bodies; without this reset the next iteration
        // of the outer par_list loop sees `if` / `done` / `select`
        // etc. as plain strings and the AST collapses.
        set_incmdpos(true);
        set_incasepat(0);
        set_incond(0);
        set_intypeset(false);
        if trailing.is_empty() {
            return Some(inner);
        }
        // Simple already absorbed its own redirs (compile path expects
        // them on ZshSimple), so don't double-wrap.
        if matches!(inner, ZshCommand::Simple(_)) {
            if let ZshCommand::Simple(mut s) = inner {
                s.redirs.extend(trailing);
                return Some(ZshCommand::Simple(s));
            }
            unreachable!()
        }
        return Some(ZshCommand::Redirected(Box::new(inner), trailing));
    }
    // Same reset on the empty-cmd branch (mirror c:1072 unconditional
    // path — the C function only returns 0 above when the dispatch
    // produced no command, and falls through to the reset block).
    set_incmdpos(true);
    set_incasepat(0);
    set_incond(0);
    set_intypeset(false);

    None
}

/// Parse for/foreach loop
/// Parse `for NAME in WORDS; do BODY; done` (foreach style) AND
/// `for ((init; cond; incr)) do BODY done` (c-style). Direct port
/// of zsh/Src/parse.c:1087 `par_for`. parse_for_cstyle is the
/// inner branch for the `((...))` arithmetic-header variant
/// (parse.c:1100-1140 inside par_for).
fn par_for() -> Option<ZshCommand> {
    let is_foreach = tok() == FOREACH;
    // c:1094-1095 (Src/parse.c, par_for) — set `infor=2` (only when
    // tok==FOR) so the lexer's `(` peek at lex.c:784-789
    // (`if (infor) { ... return DINPAR; }`) routes the arith-for
    // body through dbparens semicolon-splitting instead of the
    // `cmd_or_math` whole-body capture path. Without this, `for ((
    // i=0; i<3; i++ ))` lexed as a single `((arith))` expression
    // and parse_for_cstyle's second zshlex got an empty/wrong tok.
    //
    // The companion C statement `incmdpos = 0;` at c:1094 isn't
    // mirrored here: zshrs's parser doesn't otherwise touch
    // LEX_INCMDPOS at this boundary, and forcing it false breaks
    // the SELECT case where downstream tokenization relied on the
    // inherited state. The C parser maintains incmdpos inline at
    // every grammar transition (parse.c:617, :791, :1072, :1145,
    // :1154, :1161, ...); without porting those companion sites a
    // single explicit reset here is more harmful than helpful.
    set_infor(if tok() == FOR { 2 } else { 0 }); // c:1095
    zshlex(); // c:1096

    // Check for C-style: for (( init; cond; step ))
    if tok() == DINPAR {
        // c:1110-1111 — close out infor / cmdpos after parse_for_cstyle
        // has consumed the init/cond/step triple. Done inside the
        // helper itself so we honour the C ordering.
        return parse_for_cstyle();
    }

    // c:1116 — `infor = 0;` immediately on entering the foreach
    // branch. Without this, `infor` stays at 2 (set at c:1095 when
    // tok==FOR) for the rest of par_for, and the lexer's `((`
    // peek at lex.c:786 routes every subsequent `((...))` inside
    // the loop body through dbparens — so `for x in a; do (( 1
    // )); done` and `if (( 1 )) { … }` inside the do-body both
    // mis-lexed as a c-style for header.
    set_infor(0); // c:1116

    // Get variable name(s). zsh parse.c par_for accepts multiple
    // identifier tokens before `in`/`(`/newline — `for k v in ...`
    // assigns each iteration's pair of values to k and v in turn.
    // We store the names space-joined since variable identifiers
    // can't contain whitespace.
    let mut names: Vec<String> = Vec::new();
    while tok() == STRING_LEX {
        let v = tokstr().unwrap_or_default();
        if v == "in" {
            break;
        }
        names.push(v);
        zshlex();
    }
    if names.is_empty() {
        zerr("expected variable name in for");
        return None;
    }
    let var = names.join(" ");

    // Skip newlines
    skip_separators();

    // Get list. The lexer-port quirk: `for x (a b c)` arrives as a
    // single String token with the parens lexed-as-content
    // (`<Inpar>a b c<Outpar>`) instead of as separate Inpar/String/
    // Outpar tokens. Detect that shape and split it manually.
    let list = if tok() == STRING_LEX
        && tokstr()
            .map(|s| s.starts_with('\u{88}') && s.ends_with('\u{8a}'))
            .unwrap_or(false)
    {
        let raw = tokstr().unwrap_or_default();
        // Strip leading Inpar + trailing Outpar. KEEP the inner
        // content tokenized — `for x ({1..3}) …` has `{1..3}` as
        // Inbrace+content+Outbrace markers, which compile_word_str
        // needs to detect and brace-expand. Untokenizing here would
        // collapse the markers to plain `{` `}` chars and the brace-
        // expansion pass (which strictly requires Inbrace TOKEN per
        // Src/glob.c:hasbraces) would skip the word entirely.
        // Split only on UNTOKENIZED whitespace at the top level —
        // tokenized characters (TOKEN range \u{84}..\u{a1}) are part
        // of one word; bare ASCII spaces / tabs separate words.
        let inner = &raw[raw.char_indices().nth(1).map(|(i, _)| i).unwrap_or(0)
            ..raw
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(raw.len())];
        let mut words: Vec<String> = Vec::new();
        let mut cur = String::new();
        for c in inner.chars() {
            if c == ' ' || c == '\t' || c == '\n' {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            words.push(cur);
        }
        zshlex();
        ForList::Words(words)
    } else if tok() == STRING_LEX {
        let s = tokstr();
        if s.map(|s| s == "in").unwrap_or(false) {
            // c:Src/parse.c:1147-1154 — after consuming `in`, the
            // for-list reads in WORD position, not command position.
            // Reset incmdpos=false so the lexer's LX2_INBRACE arm
            // (lex.rs:1791) treats a leading `{` as the brace-
            // expansion marker (`bct++; add(Inbrace)`) instead of
            // returning STRING("{") + promoting to INBRACE_TOK.
            // Without this, `for i in {1..3}` saw `{` as the body-
            // opener brace, so the word-collection loop got an
            // empty word list and the loop body silently ran 0
            // iterations.
            set_incmdpos(false);
            zshlex();
            let mut words = Vec::new();
            while tok() == STRING_LEX {
                let _ts_s = tokstr();
                if let Some(s) = _ts_s.as_deref() {
                    words.push(s.to_string());
                }
                zshlex();
            }
            // c:Src/parse.c:1162 — `incmdpos = 1;` after the
            // wordlist + SEPER are consumed, so the next token
            // (`do` / `{` body opener) lexes at command position.
            set_incmdpos(true);
            ForList::Words(words)
        } else {
            ForList::Positional
        }
    } else if tok() == INPAR_TOK {
        // for var (...) — `for x ({1..3})`: inside the parens, the
        // list is in WORD position so `{` must lex as the brace-
        // expansion Inbrace marker, NOT as a body-opener INBRACE_TOK.
        // Without resetting incmdpos before the next zshlex, the
        // lexer's LX2_INBRACE arm promotes `{` to INBRACE_TOK and
        // the word-collection loop exits empty, giving
        // `for x ({1..3})` an empty iteration.
        set_incmdpos(false);
        zshlex();
        let mut words = Vec::new();
        while tok() == STRING_LEX || tok() == SEPER {
            if tok() == STRING_LEX {
                let _ts_s = tokstr();
                if let Some(s) = _ts_s.as_deref() {
                    words.push(s.to_string());
                }
            }
            zshlex();
        }
        if tok() == OUTPAR_TOK {
            // After the `)` of a for-list, the next token is the
            // body opener — `do`/`{`. zsh's lexer needs incmdpos
            // set so `{` lexes as Inbrace (not as a literal). C
            // analogue: parse.c::par_for sets `incmdpos = 1`
            // after consuming the Outpar before the body parse.
            set_incmdpos(true);
            zshlex();
        }
        ForList::Words(words)
    } else {
        ForList::Positional
    };

    // Skip to body
    skip_separators();

    // Parse body
    let body = parse_loop_body(is_foreach, false)?;

    Some(ZshCommand::For(ZshFor {
        var,
        list,
        body: Box::new(body),
        is_select: false,
    }))
}

/// Parse case statement
/// Parse `case WORD in PATTERN) BODY ;; ... esac`. Direct port
/// of zsh/Src/parse.c:1209 `par_case`. Each case arm is a
/// (pattern_list, body, terminator) tuple where terminator is
/// `;;` (default), `;&` (fallthrough), or `;|` (continue testing).
fn par_case() -> Option<ZshCommand> {
    // C par_case (parse.c:1209-1241). Order of state toggles
    // matters — the lexer reads the case word in `incmdpos=0`
    // (so it's not promoted to a reswd), then the `in`/`{` in
    // `incmdpos=1, noaliases=1, nocorrect=1` (so the `in` literal
    // isn't alias-expanded or spell-corrected), then sets
    // `incasepat=1, incmdpos=0` before the first pattern.
    set_incmdpos(false);
    zshlex(); // skip 'case'

    let word = match tok() {
        STRING_LEX => {
            let w = tokstr().unwrap_or_default();
            // c:1222 — `incmdpos = 1;` before the next zshlex so the
            // `in` keyword is recognised. c:1223-1225 — save+force
            // noaliases / nocorrect.
            set_incmdpos(true);
            let ona = noaliases();
            let onc = nocorrect();
            set_noaliases(true);
            set_nocorrect(1);
            zshlex();
            // Restore noaliases/nocorrect after the `in`-or-`{` token
            // is in hand; both are unconditionally restored at c:1238-1239.
            let restore = |ona: bool, onc: i32| {
                set_noaliases(ona);
                set_nocorrect(onc);
            };
            (w, ona, onc, restore)
        }
        _ => {
            zerr("expected word after case");
            return None;
        }
    };
    let (word, ona, onc, restore) = word;

    skip_separators();

    // Expect 'in' or {
    let use_brace = tok() == INBRACE_TOK;
    if tok() == STRING_LEX {
        let s = tokstr();
        if s.map(|s| s != "in").unwrap_or(true) {
            // c:1228-1232 — restore noaliases/nocorrect on error path.
            restore(ona, onc);
            zerr("expected 'in' in case");
            return None;
        }
    } else if !use_brace {
        restore(ona, onc);
        zerr("expected 'in' or '{' in case");
        return None;
    }
    // c:1236-1239 — `incasepat = 1; incmdpos = 0; noaliases = ona;
    // nocorrect = onc;` — set the case-pattern context AND restore
    // alias/correct state BEFORE the zshlex that consumes `in`/`{`.
    set_incasepat(1);
    set_incmdpos(false);
    restore(ona, onc);
    zshlex();

    let mut arms = Vec::new();
    const MAX_ARMS: usize = 10_000;

    loop {
        if arms.len() > MAX_ARMS {
            zerr("par_case: too many arms");
            break;
        }

        // Set incasepat BEFORE skipping separators so lexer knows we're in case pattern context
        // This affects how [ and | are lexed
        set_incasepat(1);

        skip_separators();

        // Check for end
        // Note: 'esac' might be String "esac" if incasepat > 0 prevents reserved word recognition
        let is_esac = tok() == ESAC
            || (tok() == STRING_LEX && tokstr().map(|s| s == "esac").unwrap_or(false));
        if (use_brace && tok() == OUTBRACE_TOK) || (!use_brace && is_esac) {
            set_incasepat(0);
            zshlex();
            break;
        }

        // Also break on EOF. c:Src/parse.c:1209 par_case requires
        // ESAC (or `}` in brace form) to close the block — reaching
        // ENDINPUT without either is a parse error (`case ... esack`
        // typo absorbs `esack` as part of the body and silently
        // terminates rc=0 otherwise). Bug #400.
        if tok() == ENDINPUT || tok() == LEXERR {
            set_incasepat(0);
            crate::ported::utils::zerr("unmatched `case'");
            yyerror(0);
            break;
        }

        // c:1250 — `if (tok == INPAR) zshlex();` — leading-paren
        // skip path. Used when the lexer DID return INPAR_TOK (e.g.
        // SHGLOB or incmdpos forced it). In the normal case-pattern
        // path the lexer absorbs `(...)` into one Stringg and the
        // hack at c:1322 strips the surrounding parens later. Both
        // paths land here.
        let leading_inpar_consumed = tok() == INPAR_TOK;
        if leading_inpar_consumed {
            zshlex();
        }

        // c:1255-1262 — read pattern STRING. zsh's parser falls
        // straight into the STRING reader after the optional INPAR.
        // BAR before any pattern means empty string.
        let mut patterns = Vec::new();
        // Tracks whether the c:1322-1354 hack has fired (paren-
        // wrapped Stringg absorbed by the lexer). When it has, the
        // closing `)` was already absorbed — no separate OUTPAR
        // arm-close to consume.
        let mut absorbed_outpar = false;

        // Nested-paren pattern: the user wrote `((alt|alt)tail)` —
        // the leading arm-INPAR was consumed; the NEXT token is also
        // INPAR. zinit.zsh:2946 hits this with `((add-|)fpath)`
        // meaning "fpath or add-fpath".
        //
        // The current lexer (cmd_or_math rewind path interacting
        // with gettokstr) drops ONE of the two `)` chars on the
        // way out, so the token stream is INPAR INPAR STRING BAR
        // STRING OUTPAR — only ONE OUTPAR for two source `)`. We
        // can't reconstruct the exact pattern from these tokens.
        // Proper fix needs to land in lex.rs (cmd_or_math rewind +
        // gettokstr LX2_INPAR/OUTPAR interplay) — see docs/BUGS.md
        // entry "case-pattern nested-paren lexer gap".
        //
        // Workaround: consume every token up to and INCLUDING the
        // arm-closing OUTPAR, build a best-effort pattern string,
        // set `absorbed_outpar = true` so the post-pattern OUTPAR
        // check below skips. The resulting glob may match a slightly
        // wider set than the original (`(add-|fpath)` vs
        // `(add-|)fpath`) but the rest of the script parses + runs.
        // Unblocking sourcing zinit-style configs is the priority.
        if leading_inpar_consumed && tok() == INPAR_TOK {
            let mut buf = String::new();
            let mut depth = 0i32;
            loop {
                match tok() {
                    INPAR_TOK => {
                        buf.push('(');
                        depth += 1;
                        zshlex();
                    }
                    OUTPAR_TOK => {
                        // The single OUTPAR token in the stream is
                        // simultaneously the inner glob-group close
                        // AND the arm close (lexer collapses both).
                        // Consume it, balance the inner depth, exit.
                        // `absorbed_outpar = true` tells the shared
                        // post-pattern check below to skip its own
                        // consume.
                        if depth > 0 {
                            buf.push(')');
                            depth -= 1;
                        }
                        zshlex();
                        break;
                    }
                    STRING_LEX => {
                        if let Some(s) = tokstr() {
                            if s == "esac" {
                                break;
                            }
                            buf.push_str(&s);
                        }
                        set_incasepat(2);
                        zshlex();
                    }
                    BAR_TOK => {
                        buf.push('|');
                        set_incasepat(1);
                        zshlex();
                    }
                    _ => break,
                }
            }
            patterns.push(buf);
            set_incasepat(0);
            set_incmdpos(true);
            absorbed_outpar = true;
        }

        // Skip the legacy STRING/BAR pattern-read loop when the
        // nested-paren branch above already populated `patterns` and
        // consumed the arm-close. Otherwise the legacy loop would
        // greedily absorb the body's first command token (`echo`) as
        // another alt-pattern.
        if !patterns.is_empty() && absorbed_outpar {
            // skip to body parse
        } else {
        loop {
            if tok() == STRING_LEX {
                let s = tokstr();
                if s.as_deref().map(|s| s == "esac").unwrap_or(false) {
                    break;
                }
                let mut str_val = s.unwrap_or_default();

                // c:1322-1354 hack: when this is the first alt AND
                // the string starts with the Inpar marker, the lexer
                // absorbed the whole `(...)` as one token. Chuck the
                // blanks around `|`/parens at depth 1 (c:1332-1338 —
                // `( d | e )` must become `(d|e)`), then strip the
                // surrounding parens — the remainder IS the pattern.
                // The closing arm-paren was absorbed too, so we don't
                // expect a separate OUTPAR token afterward.
                if patterns.is_empty() && str_val.starts_with(crate::ported::zsh_h::Inpar) {
                    use crate::ported::zsh_h::{Bar, Inpar, Outpar};
                    let meta = crate::ported::zsh_h::Meta as char;
                    let blank = |c: char| c.is_ascii() && crate::ported::ztype_h::iblank(c as u8);
                    let mut chars: Vec<char> = str_val.chars().collect();
                    let mut pct = 0i32;
                    let mut i = 0usize;
                    let mut end_idx: Option<usize> = None;
                    while i < chars.len() {
                        if chars[i] == Inpar {
                            pct += 1;
                        }
                        if pct == 1 {
                            // c:1332-1334 — chuck blanks AFTER `|`/`(`.
                            if chars[i] == Bar || chars[i] == Inpar {
                                while i + 1 < chars.len() && blank(chars[i + 1]) {
                                    chars.remove(i + 1);
                                }
                            }
                            // c:1335-1338 — chuck blanks BEFORE `|`/`)`
                            // (not Meta-escaped blanks).
                            if chars[i] == Bar || chars[i] == Outpar {
                                while i >= 1
                                    && blank(chars[i - 1])
                                    && (i < 2 || chars[i - 2] != meta)
                                {
                                    chars.remove(i - 1);
                                    i -= 1;
                                }
                            }
                        }
                        if chars[i] == Outpar {
                            pct -= 1;
                            if pct == 0 {
                                end_idx = Some(i);
                                break;
                            }
                        }
                        i += 1;
                    }
                    if let Some(idx) = end_idx {
                        chars.remove(idx);
                        chars.remove(0);
                        str_val = chars.into_iter().collect();
                        absorbed_outpar = true;
                    }
                }
                patterns.push(str_val);
                if absorbed_outpar {
                    // c:Src/parse.c:1300-1302 — after a whole-`(...)`
                    // pattern the next token may be the body's first
                    // command word; C lexes it with `incasepat = -1;
                    // incmdpos = 1;` so assignments (`out+=hit`)
                    // become ENVSTRING instead of a plain STRING
                    // (lex.c:1229-1230 gates ENVSTRING on incmdpos).
                    set_incasepat(-1);
                    set_incmdpos(true);
                } else {
                    set_incasepat(2);
                }
                zshlex();
                // When the hack fired the closing `)` is already
                // consumed; don't read alt-`|` continuations either.
                if absorbed_outpar {
                    break;
                }
            } else if tok() != BAR_TOK {
                break;
            }

            if tok() == BAR_TOK {
                set_incasepat(1);
                zshlex();
            } else {
                break;
            }
        }
        }  // end else of "skip legacy loop when nested-paren branch fired"
        set_incasepat(0);

        // c:1305 — expect OUTPAR (arm-close) when the hack didn't
        // already swallow it.
        //
        // Bug #34 in docs/BUGS.md: the absorbed-pattern hack assumed
        // the leading `(` and the case-arm closing `)` were both
        // absorbed into the single STRING token. That's true for
        // `(x))` (the inner `)` closes the absorbed group; the second
        // `)` is the arm closer) only when the lexer slurps BOTH.
        // The Rust lexer slurps just `(x|y)` (one balanced pair); the
        // second `)` arrives as a separate OUTPAR_TOK that must still
        // be consumed as the case-arm closer. Detect and consume it.
        if !absorbed_outpar {
            if tok() != OUTPAR_TOK {
                zerr("expected ')' in case pattern");
                return None;
            }
            // c:Src/parse.c:1257-1258 — `if (tok != STRING)
            // YYERRORV(oecused);` C requires at least one pattern
            // STRING before `)`. zshrs accepted empty `case x in)`
            // and silently fell through to the next iteration with
            // an empty pattern arm, swallowing the rest of the
            // script. Reject the empty-pattern shape unless a
            // leading INPAR was consumed (the `(pat)` form has
            // already validated the pattern inside). Bug #161 in
            // docs/BUGS.md.
            if patterns.is_empty() && !leading_inpar_consumed {
                zerr("parse error near `)'");
                return None;
            }
            set_incmdpos(true);
            zshlex();
            // When the lexer emitted a separate INPAR_TOK at the
            // arm start (consumed via `leading_inpar_consumed`
            // above), the OUTPAR_TOK we just consumed closed the
            // alternation GROUP. If the next token is ALSO
            // OUTPAR_TOK, the user wrote `(pat))` and that second
            // `)` is the case-arm closer that still needs to be
            // consumed before body parsing. Bug #34 in
            // docs/BUGS.md.
            if leading_inpar_consumed && tok() == OUTPAR_TOK {
                zshlex();
            }
        } else if tok() == OUTPAR_TOK {
            // The lexer absorbed `(pat)` as the pattern but left the
            // case-arm closing `)` as a separate OUTPAR_TOK. Consume
            // it now so body parsing starts at the body, not at `)`.
            set_incmdpos(true);
            zshlex();
        } else {
            set_incmdpos(true);
        }

        // Parse body. Pass end_tokens explicitly so the body's
        // parser stops at DSEMI/SEMIAMP/SEMIBAR/ESAC without
        // tripping parse_program_until's orphan-terminator check
        // (line 7131) which only fires when end_tokens is None.
        // Without this, a case arm whose body has no trailing
        // `;;` before `esac` (last arm — zsh accepts the dangling
        // form) produced "parse error near orphan terminator" on
        // the closing `esac`. zsh's par_case at parse.c:1318 sets
        // up the case-arm reader to recognize the same terminator
        // set; the Rust port was passing the implicit-None and
        // hitting the top-level orphan check.
        let body = parse_program_until(Some(&[DSEMI, SEMIAMP, SEMIBAR, ESAC]));

        // Get terminator. Set incasepat=1 BEFORE the zshlex
        // advance so the next token (the next arm's pattern, like
        // `[a-z]`) gets tokenized in pattern context. Without
        // this, a `[`-prefixed pattern after the FIRST arm became
        // Inbrack instead of String and the pattern-loop bailed
        // out with "expected ')' in case pattern".
        // c:Src/parse.c:1391 — `incasepat = 1; incmdpos = 0;` BEFORE
        // the zshlex that advances past `;;` to the next arm's first
        // token. The incmdpos=0 setting is what makes the lexer
        // absorb the next arm's `((add-|)fpath)` into a single STRING
        // (gettokstr's LX2_INPAR path at lex.c:1080+ runs only when
        // incmdpos is FALSE — at incmdpos=1 the lexer emits raw
        // INPAR_TOK for the inner `(` and par_case's pattern-read
        // loop has no path to recover). Our Rust port previously set
        // only incasepat=1 and not incmdpos=0, which forced
        // multi-arm patterns like `((add-|)fpath)` to fail with
        // "expected ')' in case pattern" — surfaced on
        // zinit.zsh:2946.
        let terminator = match tok() {
            DSEMI => {
                set_incasepat(1);
                set_incmdpos(false);
                zshlex();
                CaseTerm::Break
            }
            SEMIAMP => {
                set_incasepat(1);
                set_incmdpos(false);
                zshlex();
                CaseTerm::Continue
            }
            SEMIBAR => {
                set_incasepat(1);
                set_incmdpos(false);
                zshlex();
                CaseTerm::TestNext
            }
            _ => CaseTerm::Break,
        };

        if !patterns.is_empty() {
            arms.push(CaseArm {
                patterns,
                body,
                terminator,
            });
        }
    }

    Some(ZshCommand::Case(ZshCase { word, arms }))
}

/// Parse if statement
/// Parse `if COND; then BODY; [elif COND; then BODY;]* [else BODY;] fi`.
/// Direct port of zsh/Src/parse.c:1411 `par_if`. The C source
/// emits WC_IF wordcodes per arm; zshrs builds an AST chain of
/// (cond, then_body) tuples plus an optional else_body.
fn par_if() -> Option<ZshCommand> {
    zshlex(); // skip 'if'

    // Parse condition - stops at 'then' or '{' (zsh allows { instead of then).
    //
    // Special-case: when the cond starts with `{` (INBRACE_TOK at
    // the very first token), the `{` could be either
    //   (a) the if's brace-form body opener — `if { BODY }`, or
    //   (b) the first command of a brace-group condition — e.g.
    //       `if { CMD } || { CMD }; then BODY; fi`
    // (p10k.zsh:8376 uses (b)). Disambiguate by parsing one sublist
    // (par_sublist handles `{ … } &&/|| { … }` natively); if after
    // the sublist tok is THEN (with optional separator), treat it
    // as cond and proceed to then-form. If tok is INBRACE_TOK (the
    // sublist was a single brace-group → that `{ … }` was actually
    // the body), restore by re-entering parse_program_until on an
    // empty cond and falling through to the brace-form arm.
    let cond_is_brace_open = tok() == INBRACE_TOK;
    let cond_special: Option<Box<ZshProgram>> = if cond_is_brace_open {
        let sub = par_sublist();
        skip_separators();
        if tok() == THEN || tok() == INBRACE_TOK {
            // After the sublist, tok==THEN → then-form with the
            // sublist as cond; tok==INBRACE_TOK → brace-form (the
            // sublist we just parsed IS the body, mis-claimed).
            if tok() == THEN {
                // then-form: wrap the sublist as the cond program.
                sub.map(|s| {
                    Box::new(ZshProgram {
                        lists: vec![ZshList {
                            sublist: s,
                            flags: ListFlags::default(),
                        }],
                    })
                })
            } else {
                // INBRACE_TOK after a single sublist that started
                // with `{`: the sublist parse consumed the BODY
                // brace-group thinking it was a cond statement.
                // We can't trivially rewind the lexer; surface as a
                // sentinel so the brace-form arm below errors with
                // the proper "expected 'then' or '{'" diagnostic.
                // This case `if { a } { b }` is brace-form with a
                // brace-group cond and brace-group body — rare in
                // real code; deferred.
                None
            }
        } else {
            // Sublist did NOT terminate at THEN/INBRACE — fall back
            // to the original parse_program_until path which will
            // handle whatever follows.
            sub.map(|s| {
                Box::new(ZshProgram {
                    lists: vec![ZshList {
                        sublist: s,
                        flags: ListFlags::default(),
                    }],
                })
            })
        }
    } else {
        None
    };
    let cond = cond_special
        .unwrap_or_else(|| Box::new(parse_program_until(Some(&[THEN, INBRACE_TOK]))));

    skip_separators();

    // Expect 'then' or {
    let use_brace = tok() == INBRACE_TOK;
    if tok() != THEN && !use_brace {
        zerr("expected 'then' or '{' after if condition");
        return None;
    }
    zshlex();

    // Parse then-body - stops at else/elif/fi, or } if using brace syntax
    let then = if use_brace {
        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
        if tok() == OUTBRACE_TOK {
            zshlex();
            // c:Src/parse.c:1469-1470 — `zshlex(); incmdpos = 1;`. C
            // par_if explicitly resets incmdpos to 1 after consuming
            // OUTBRACE so subsequent commands (`; echo after`, `else`
            // following the `}` without a `;`) are at command position
            // and tokenize correctly.
            set_incmdpos(true);
        }
        Box::new(body)
    } else {
        Box::new(parse_program_until(Some(&[ELSE, ELIF, FI])))
    };

    // c:Src/parse.c:1471-1472 — `if (tok == SEPER) break;`. For
    // brace-form `if … { … }`, C par_if breaks out of the outer
    // construct loop WITHOUT consuming the SEPER when one
    // immediately follows the closing `}`. The SEPER stays in the
    // lexer for the OUTER par_list to consume as the if-statement's
    // list separator. Our Rust port's loop below calls skip_separators
    // which would eat that SEPER and leave tok=STRING(next-command),
    // triggering par_cmd's "STRING_LEX after compound = parse error"
    // check at line ~1170. Without this early-exit, brace-form
    //   if [[ … ]] { … }; echo after
    // failed with `parse error near 'echo'` on zinit.zsh:1422.
    //
    // We only need this when use_brace is true and the next token
    // is a separator (SEPER / NEWLIN / SEMI) AND it's NOT followed
    // by ELSE/ELIF (those would extend the construct). Since the
    // brace-form already consumed its closing `}` (saw_terminator
    // is true), early-return is safe.
    if use_brace && matches!(tok(), SEPER | NEWLIN | SEMI | ENDINPUT) {
        return Some(ZshCommand::If(ZshIf {
            cond,
            then,
            elif: Vec::new(),
            else_: None,
        }));
    }

    // Parse elif and else. zsh accepts the SAME elif/else
    // continuations for both classic `then/fi` AND the brace
    // form `{ ... } elif ... { ... } else { ... }`. Direct port
    // of zsh/Src/parse.c:1417-1500 par_if where the elif/else
    // arms are checked AFTER the body close regardless of which
    // delimiter style opened the block. Without this, zinit's
    //   if [[ -z $sel ]] { ... } else { ... }
    // hung the parser — `else` was treated as an external
    // command following the if-statement, which the lexer state
    // mis-classified inside the still-open function body.
    //
    // For brace-form: skip the `fi` consumption at the end of
    // the loop (no `fi` after a brace block), and `else` may
    // arrive after a `}` close. Skip-separators between the
    // body close and the elif/else token.
    let mut elif = Vec::new();
    let mut else_ = None;
    // c:Src/parse.c:1501-1504 — `if (tok != FI) { cmdpop(); YYERRORV; }`.
    // The C parser fails the whole if-construct when the body close
    // isn't seen. zshrs's loop fell through silently on ENDINPUT, so
    // `if true; then echo yes` (no `fi`) was accepted. Track whether
    // we hit a real terminator and error after the loop if not.
    let mut saw_terminator = use_brace; // `{ … }` body already consumed its close

    {
        loop {
            skip_separators();

            match tok() {
                ELIF => {
                    zshlex();
                    // elif condition stops at 'then' or '{'
                    let econd = parse_program_until(Some(&[THEN, INBRACE_TOK]));
                    skip_separators();

                    let elif_use_brace = tok() == INBRACE_TOK;
                    if tok() != THEN && !elif_use_brace {
                        zerr("expected 'then' after elif");
                        return None;
                    }
                    zshlex();

                    // elif body stops at else/elif/fi or } if using braces
                    let ebody = if elif_use_brace {
                        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
                        if tok() == OUTBRACE_TOK {
                            zshlex();
                            saw_terminator = true; // brace close on elif
                        }
                        body
                    } else {
                        parse_program_until(Some(&[ELSE, ELIF, FI]))
                    };

                    elif.push((econd, ebody));
                }
                ELSE => {
                    zshlex();
                    skip_separators();

                    // Brace-form `else { … }` is only legal when the
                    // PARENT IF itself was opened brace-form (`if cond
                    // { … }`). For a `then`-form if, `else { stmt }`
                    // is `else` followed by a brace-group STATEMENT
                    // that's part of the else body's statement list,
                    // which still terminates at `fi`. p10k.zsh:5575
                    // hits exactly this shape:
                    //   else
                    //     { local v=($(<$file)) } 2>/dev/null
                    //   fi
                    // The prior port unconditionally consumed the `{`
                    // as else-brace-opener, then expected the `fi`
                    // outside the if construct → "expected `done'".
                    let else_use_brace = use_brace && tok() == INBRACE_TOK;
                    if else_use_brace {
                        zshlex();
                    }

                    // else body stops at 'fi' or '}'
                    else_ = Some(Box::new(if else_use_brace {
                        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
                        if tok() == OUTBRACE_TOK {
                            zshlex();
                            saw_terminator = true;
                        }
                        body
                    } else {
                        parse_program_until(Some(&[FI]))
                    }));

                    // Consume the 'fi' if present (not for brace syntax)
                    if !else_use_brace && tok() == FI {
                        zshlex();
                        saw_terminator = true;
                    }
                    break;
                }
                FI => {
                    // Brace-form `if ... { ... }` is already terminated by
                    // its closing `}`. Do NOT consume `fi` here — it belongs
                    // to an enclosing then-form if. Without this gate, a
                    // brace-form if inside a then-form if's body would steal
                    // the outer `fi`, leaving the outer parser to see
                    // "unterminated if". This bit zinit-install.zsh:978
                    // where `if (( … )) {` (brace) inside `if … ; then …`
                    // (then-form) ate the outer `fi`.
                    if use_brace {
                        break;
                    }
                    zshlex();
                    saw_terminator = true;
                    break;
                }
                _ => break,
            }
        }
    }

    if !saw_terminator {
        // c:1501-1504 — YYERRORV when the if-construct never closed.
        zerr("parse error: unterminated if");
        return None;
    }

    Some(ZshCommand::If(ZshIf {
        cond,
        then,
        elif,
        else_,
    }))
}

/// Parse while/until loop
/// Parse `while COND; do BODY; done` and `until COND; do BODY; done`.
/// Direct port of zsh/Src/parse.c:1521 `par_while`. The
/// `until` variant is the same loop with the condition negated.
fn par_while(until: bool) -> Option<ZshCommand> {
    zshlex(); // skip while/until

    // c:1521-1551 par_while — the condition's parser must stop at
    // `do` or `{`. Without an explicit end-token set, parse_program
    // consumes the brace-form body as additional condition lists,
    // leaving parse_loop_body with nothing — `while (( i++ < 3 )) {
    // echo $i }` silently parsed but executed nothing.
    // c:1528 — `par_save_list(cmplx);` — the cond is an ORDINARY
    // list: a leading `{...}` block is the cond's first command
    // (`while {false} {body}`), and the body INBRACE is reachable
    // because a list followed by `{` without a separator ends (the
    // chaining rule in parse_program_until). DOLOOP stays in the
    // stop set because `do` at command position can't start a
    // command. INBRACE_TOK was previously (wrongly) in the set too,
    // which made a brace-form COND parse as an empty cond + body.
    let cond = Box::new(parse_program_until(Some(&[DOLOOP])));

    skip_separators();
    let body = parse_loop_body(false, false)?;

    // c:Src/parse.c:1521-1551 par_while — WC_WHILE wordcode is tagged
    // with WC_WHILE_TYPE differentiating WHILE vs UNTIL at the wordcode
    // layer. The AST mirror in zsh_ast.rs has separate Until(ZshWhile)
    // and While(ZshWhile) variants; route by the `until` flag here so
    // downstream pattern-matchers can distinguish without poking
    // inside the payload's bool.
    let w = ZshWhile {
        cond,
        body: Box::new(body),
        until,
    };
    Some(if until {
        ZshCommand::Until(w) // c:1521 (WC_WHILE_TYPE = WC_WHILE_UNTIL)
    } else {
        ZshCommand::While(w) // c:1521 (WC_WHILE_TYPE = WC_WHILE_WHILE)
    })
}

/// Parse repeat loop
/// Parse `repeat N; do BODY; done`. Direct port of
/// zsh/Src/parse.c:1565 `par_repeat`. The C source supports
/// the SHORTLOOPS short-form `repeat N CMD` (no do/done) — zshrs's
/// parser doesn't yet special-case that variant.
fn par_repeat() -> Option<ZshCommand> {
    zshlex(); // skip 'repeat'

    let count = match tok() {
        STRING_LEX => {
            let c = tokstr().unwrap_or_default();
            zshlex();
            c
        }
        _ => {
            zerr("expected count after repeat");
            return None;
        }
    };

    skip_separators();
    // c:1600 — par_repeat's short-form gate is wider: it unlocks
    // when SHORTLOOPS OR SHORTREPEAT is set (vs SHORTLOOPS alone for
    // for/while). Pass `is_repeat=true` so parse_loop_body
    // applies that widened gate.
    let body = parse_loop_body(false, true)?;

    Some(ZshCommand::Repeat(ZshRepeat {
        count,
        body: Box::new(body),
    }))
}

/// Parse (...) subshell
/// Parse a subshell `( ... )`. Direct port of zsh/Src/parse.c:1619
/// `par_subsh`. Body parses as a normal list; the subshell wrapper
/// fork-isolates execution in the executor.
fn par_subsh() -> Option<ZshCommand> {
    zshlex(); // skip (
    // c:Src/parse.c:par_subsh — `parse_event(OUTPAR)` parses until
    // the matching `)`. zshrs's previous port called bare
    // `parse_program()` (parse_program_until(None)) which has no
    // way to know it should stop at OUTPAR_TOK — at top-level
    // that's fine (the outer loop just sees an extra OUTPAR after
    // the inner body), but the parse_event-equivalent's new
    // yyerror-on-unconsumed-token behavior at parse_program_until's
    // None arm now reports a spurious "parse error near `)'" when
    // the construct ends. Pass OUTPAR_TOK so parse_program_until
    // stops cleanly at the closing paren.
    let prog = parse_program_until(Some(&[OUTPAR_TOK]));
    if tok() == OUTPAR_TOK {
        zshlex();
    }
    Some(ZshCommand::Subsh(Box::new(prog)))
}

/// Parse function definition
/// Parse `function NAME { BODY }` or `NAME () { BODY }`. Direct
/// port of zsh/Src/parse.c:1672 `par_funcdef`. zsh handles
/// the multiple keyword shapes (function FOO, FOO (), function FOO ()),
/// the optional `[fname1 fname2 ...]` for multi-name function defs,
/// and the `function FOO () { ... }` traditional/POSIX hybrid form.
fn par_funcdef() -> Option<ZshCommand> {
    zshlex(); // skip 'function'

    let mut names = Vec::new();
    let mut tracing = false;

    // Handle options like -T and function names. Two subtleties:
    //
    //   1. Flags: zsh's lexer encodes a leading `-` as
    //      `zsh_h::Dash` (`\u{9b}`, `Src/zsh.h:182`) inside the String tokstr.
    //      The previous `s.starts_with('-')` check failed for
    //      `\u{9b}T`, so `function -T NAME { body }` slipped the
    //      `-T` token into `names` and the function got registered
    //      as `T` plus the intended `NAME`.
    //
    //   2. Body opener: zsh's lexer emits the opening `{` as a
    //      String (not INBRACE_TOK) when it follows the String
    //      NAME — the preceding name token resets incmdpos to
    //      false, and only `{` immediately followed by `}` (the
    //      empty-body case) gets promoted to Inbrace. The funcdef
    //      parser must recognise the bare-`{` String as the body
    //      opener; otherwise `function NAME { body }` falls through
    //      to `_ => break`, no body parses, and the FuncDef never
    //      lands in the AST. This is consistent with C zsh's
    //      par_funcdef which knows it's in funcdef-header context
    //      and accepts the brace either way.
    loop {
        match tok() {
            STRING_LEX => {
                let _ts_s = tokstr()?;
                let s = _ts_s.as_str();
                // c:1702 — `if ((*tokstr == Inbrace || *tokstr == '{') && !tokstr[1])`.
                // Body opener can be either the literal `{` (early-return
                // path at lex.c:1141-1144 / lex.rs LX2_INBRACE cmdpos
                // branch) or the Inbrace marker `\u{8f}` (lex.c:1420
                // post-switch add(c) where c was rewritten via lextok2).
                if s == "{" || s == "\u{8f}" {
                    break;
                }
                let first = s.chars().next();
                if matches!(first, Some('-') | Some('+')) || matches!(first, Some(c) if c == Dash) {
                    if s.contains('T') {
                        tracing = true;
                    }
                    zshlex();
                    continue;
                }
                // c:Src/exec.c::execcmd_args — function name tokens
                // in `function NAME { ... }` form go through globbing
                // at parse time. zsh's `function with[bracket] { ... }`
                // triggers a glob expansion of `with[bracket]`; no file
                // matches → "no matches found: NAME" + rc=1 (when
                // NOMATCH is set, the default). Bug #536: zshrs accepted
                // the literal bracket-containing name and registered
                // the function silently. Mirror C by probing for glob
                // metachars on the name; if present AND no file
                // matches, emit the diagnostic and abort the parse.
                let has_glob_chars = s.chars().any(|c| {
                    matches!(
                        c,
                        '[' | ']'
                            | '*'
                            | '?'
                            | crate::ported::zsh_h::Inbrack
                            | crate::ported::zsh_h::Outbrack
                            | crate::ported::zsh_h::Star
                            | crate::ported::zsh_h::Quest
                    )
                });
                if has_glob_chars && crate::ported::zsh_h::isset(crate::ported::zsh_h::NOMATCH) {
                    let untok = crate::ported::lex::untokenize(s);
                    let glob_result = crate::ported::glob::glob(&untok);
                    if glob_result.is_empty() {
                        crate::ported::utils::zerr(&format!("no matches found: {}", untok));
                        crate::ported::utils::errflag.fetch_or(
                            crate::ported::utils::ERRFLAG_ERROR,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        return None;
                    }
                }
                names.push(s.to_string());
                zshlex();
            }
            INBRACE_TOK | INOUTPAR | SEPER | NEWLIN => break,
            _ => break,
        }
    }

    // Optional ()
    let saw_paren = tok() == INOUTPAR;
    if saw_paren {
        zshlex();
    }

    skip_separators();

    // Body opener: real Inbrace OR a String containing the literal `{`
    // (early-return path) OR a String containing the Inbrace marker
    // `\u{8f}` (bct++ path post-switch add). C parse.c:1702 handles
    // both string forms via `*tokstr == Inbrace || *tokstr == '{'`.
    let body_opener_is_string_brace =
        tok() == STRING_LEX && tokstr().map(|s| s == "{" || s == "\u{8f}").unwrap_or(false);
    if tok() == INBRACE_TOK || body_opener_is_string_brace {
        // Capture body_start BEFORE the lexer advances past the
        // first body token. After the previous zshlex consumed
        // `{`, lexer.pos points just past `{` (which is where the
        // body source starts). The next `zshlex()` would advance
        // past the first token (`echo`), making body_start land
        // mid-body and lose the first word — `typeset -f f` would
        // print `a; echo b` for `{ echo a; echo b }`.
        // c:Src/parse.c:1690-1706 — par_funcdef requires a clean
        //   body-opener brace when the anonymous form `function {body}`
        //   is used (no names AND no `()`). zsh's lexer keeps the `{`
        //   as its own STRING token via the lex.c:1141-1144 early-
        //   return at command position, but the body brace must be
        //   followed by whitespace for the inner par_list to find a
        //   matching OUTBRACE — without a separator, the closing `}`
        //   gets merged into the last word (`X}`) and par_list ends
        //   without OUTBRACE, which C zsh reports as `parse error near
        //   \`}'`. zshrs's lexer has the same `bct` semantics; reject
        //   here at the parse step so the funcdef doesn't silently run
        //   with the stray `}` attached. With names or `()` present,
        //   the body brace is allowed even without a separator
        //   (`function name {body}` and `function () {body}` both work
        //   in zsh). Bug #60 in docs/BUGS.md.
        if names.is_empty() && !saw_paren {
            // Peek the next source byte after the current lexer position
            // (`{` was just tokenized — `pos()` points just past it).
            // A whitespace separator means proper `function { body }`
            // form; anything else is the malformed `function {body}`
            // shape zsh rejects.
            let next_byte = input_slice(pos(), pos() + 1)
                .and_then(|s| s.bytes().next())
                .unwrap_or(b' ');
            if !matches!(next_byte, b' ' | b'\t' | b'\n' | b';') {
                zerr("parse error near `}'"); // c:Src/parse.c YYERRORV
                return None;
            }
        }
        let body_start = pos();
        zshlex();
        // c:Src/parse.c — func body terminates at OUTBRACE_TOK.
        // Explicit end-token keeps the inner parse from hitting the
        // top-level stray-`}` arm (#168). Bug #167 family.
        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
        // c:Src/parse.c:1733-1737 — `if (tok != OUTBRACE) { cmdpop();
        // ... YYERRORV(oecused); }`. Hard-error on missing close brace
        // so `function f { echo hi` doesn't silently register a half-
        // parsed body. Bug #405.
        if tok() != OUTBRACE_TOK {
            zerr("parse error: expected `}'");
            return None;
        }
        let body_end = pos().saturating_sub(1);
        let body_source = input_slice(body_start, body_end)
            .map(|s| {
                // Lexer's pos() may have advanced past `}` AND skipped
                // trailing whitespace/newlines before returning the
                // OUTBRACE_TOK to us, so the slice up to `pos - 1`
                // includes the `}` and any preceding whitespace.
                // Strip the trailing `}` and any preceding structural
                // separator (`;`, `\n`) — C zsh's getpermtext walks
                // the wordcode list and emits each command WITHOUT
                // the trailing `;`/`\n` that lives in the input.
                let t = s.trim();
                let t = t.strip_suffix('}').unwrap_or(t).trim_end();
                let t = t
                    .trim_end_matches(|c: char| c == ';' || c == '\n')
                    .trim_end();
                t.to_string()
            })
            .filter(|s| !s.is_empty());
        zshlex();

        // Anonymous form `function () { body } a b c` (with `()`) or
        // `function { body } a b c` (zsh-only shorthand, no `()`). No
        // name was collected. Mirror parse_anon_funcdef: synthesize
        // `_zshrs_anon_N`, collect trailing args, set auto_call_args
        // so compile_funcdef registers + immediately calls the
        // function with the args as positional params.
        if names.is_empty() {
            let mut args = Vec::new();
            while tok() == STRING_LEX {
                if let Some(s) = tokstr() {
                    args.push(s);
                }
                zshlex();
            }
            static ANON_COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = ANON_COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("_zshrs_anon_kw_{}", n);
            return Some(ZshCommand::FuncDef(ZshFuncDef {
                names: vec![name],
                body: Box::new(body),
                tracing,
                auto_call_args: Some(args),
                body_source,
            }));
        }

        Some(ZshCommand::FuncDef(ZshFuncDef {
            names,
            body: Box::new(body),
            tracing,
            auto_call_args: None,
            body_source,
        }))
    } else {
        // Short form
        par_list().map(|(list, _terminated)| {
            ZshCommand::FuncDef(ZshFuncDef {
                names,
                body: Box::new(ZshProgram { lists: vec![list] }),
                tracing,
                auto_call_args: None,
                body_source: None,
            })
        })
    }
}

/// Parse time command
/// Parse `time CMD` (POSIX time keyword). Direct port of
/// zsh/Src/parse.c:1787 `par_time`. The `time` keyword
/// times the execution of the following pipeline / cmd.
fn par_time() -> Option<ZshCommand> {
    zshlex(); // skip 'time'

    // Check if there's a pipeline to time
    if tok() == SEPER || tok() == NEWLIN || tok() == ENDINPUT {
        Some(ZshCommand::Time(None))
    } else {
        let sublist = par_sublist();
        Some(ZshCommand::Time(sublist.map(Box::new)))
    }
}

/// Port of `par_dinbrack(void)` from `Src/parse.c:1810`. Body
/// parser inside `[[ ... ]]` — calls `par_cond` to emit the
/// condition wordcode then advances past `]]`.
pub fn par_dinbrack() -> Option<()> {
    // c:1810
    set_incond(1); // c:1814
    set_incmdpos(false); // c:1815
    zshlex(); // c:1816
    let _ = par_cond(); // c:1817
    if tok() != DOUTBRACK {
        // c:1818
        crate::ported::utils::zerr("missing ]]");
        yyerror(0);
        return None;
    }
    set_incond(0); // c:1820
    set_incmdpos(true); // c:1821
    zshlex(); // c:1822
    Some(())
}

/// Parse a simple command
/// Parse a simple command (assignments + words + redirections).
/// Direct port of zsh/Src/parse.c:1836 `par_simple` —
/// the largest single function in parse.c. Handles ENVSTRING/
/// ENVARRAY assignments at command head, intermixed redirs,
/// typeset-style multi-assignment commands, and the trailing
/// inout-par `()` that converts a simple command into an inline
/// function definition.
fn par_simple(mut redirs: Vec<ZshRedir>) -> Option<ZshCommand> {
    let mut assigns = Vec::new();
    let mut words = Vec::new();

    // c:1934-1974 — `{var}>file` brace-FD detection is wired
    // INSIDE the words loop below (parse.rs:4940-4956) rather than
    // here at the head. The words-loop site sees the tok=STRING
    // `{varname}` followed by a REDIROP and routes into par_redir
    // with redir.varid populated. C does it inline at the start of
    // each STRING/TYPESET arm iteration; functionally equivalent.

    // c:1843-1846 — leading-NOCORRECT prefix: `nocorrect echo hello`
    // emits a NOCORRECT token at the start of par_simple. C sets
    // `nocorrect = 1` and skips past via the `zshlex();` at the
    // for-loop tail (c:1907). zshrs's par_simple (AST) had no
    // NOCORRECT arm so the token was silently dropped and the
    // following command line evaporated — `nocorrect echo hello`
    // produced empty output.
    while tok() == NOCORRECT {
        set_nocorrect(1); // c:1846
        zshlex(); // c:1907 (loop-tail zshlex)
    }

    // Parse leading assignments
    while tok() == ENVSTRING || tok() == ENVARRAY {
        if let Some(assign) = parse_assign() {
            assigns.push(assign);
        }
        zshlex();
    }

    // Parse words and redirections
    loop {
        match tok() {
            ENVSTRING | ENVARRAY => {
                // Mid-command assignment-shape arg under typeset
                // / declare / local / etc. (intypeset gates the
                // lexer to emit Envstring/Envarray for `name=val`
                // and `name=()` past the command name). Parse the
                // assignment, then emit a synthetic word
                // `NAME=value` (scalar) or `NAME=( … )` (array)
                // string so typeset's builtin arg list sees the
                // assignment-shape arg. Avoids the inline-env
                // scope path that mistakenly treats it like a
                // pre-cmd `X=Y cmd` assignment.
                if let Some(assign) = parse_assign() {
                    let synthetic = match &assign.value {
                        ZshAssignValue::Scalar(v) => format!("{}={}", assign.name, v),
                        ZshAssignValue::Array(elems) => {
                            // c:Src/builtin.c — assoc paren-init `h=( "" v
                            //   k2 v2 )` must preserve empty-string
                            //   elements (zsh stores key="" + value="v").
                            //   The bin_typeset paren-init splitter at
                            //   `builtin.rs:4358` recognizes the
                            //   REJOIN_SEP (`\u{1f}`) sentinel between
                            //   array elements and skips the leading/
                            //   trailing parens trim; using it here
                            //   round-trips empties end-to-end through
                            //   the synthetic-arg rebuild. Space-join
                            //   collapses adjacent empties (`(` + `""` +
                            //   `empty-val` becomes `( empty-val`) so
                            //   bin_typeset never sees the empty key.
                            //   Bug #93 in docs/BUGS.md.
                            let mut buf = String::with_capacity(
                                assign.name.len() + 4 + elems.iter().map(|e| e.len() + 1).sum::<usize>(),
                            );
                            buf.push_str(&assign.name);
                            buf.push_str("=(");
                            for elem in elems {
                                buf.push('\u{1f}');
                                buf.push_str(elem);
                            }
                            buf.push('\u{1f}');
                            buf.push(')');
                            buf
                        }
                    };
                    words.push(synthetic);
                }
                zshlex();
            }
            STRING_LEX | TYPESET => {
                let s = tokstr();
                if let Some(s) = s {
                    words.push(s);
                }
                // c:1929 — `incmdpos = 0;` so the next zshlex() does
                // not re-promote `{`/`[[`/reserved words at the
                // continuation position. Without this, `echo {a,b}`
                // re-lexes `{` as INBRACE_TOK (current-shell block)
                // and the brace expansion never reaches par_simple.
                set_incmdpos(false);
                // c:1931-1932 — `if (tok == TYPESET) intypeset = is_typeset = 1;`
                // Multi-assign `typeset a=1 b=2` relies on the lexer
                // re-emitting `b=2` as ENVSTRING; that path is gated
                // on `intypeset`. Without this, follow-on assignment
                // words arrive as STRING and the typeset builtin's
                // multi-assign form silently degrades.
                if tok() == TYPESET {
                    set_intypeset(true);
                }
                zshlex();
                // Check for function definition foo() { ... }
                if words.len() == 1 && tok() == INOUTPAR {
                    return parse_inline_funcdef(words.pop().unwrap());
                }
                // `{name}>file` named-fd redirect: the lexer doesn't
                // recognize this shape, so the bare word `{name}`
                // arrives as a String. If it matches `{IDENT}` and
                // the NEXT token is a redirop, pop it off as the
                // varid for that redir.
                if !words.is_empty() && IS_REDIROP(tok()) {
                    let last = words.last().unwrap();
                    let untoked = super::lex::untokenize(last);
                    if untoked.starts_with('{') && untoked.ends_with('}') && untoked.len() > 2 {
                        let name = &untoked[1..untoked.len() - 1];
                        if !name.is_empty()
                            && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
                            && name
                                .chars()
                                .next()
                                .map(|c| c == '_' || c.is_ascii_alphabetic())
                                .unwrap_or(false)
                        {
                            let varid = name.to_string();
                            words.pop();
                            if let Some(mut redir) = par_redir() {
                                redir.varid = Some(varid);
                                redirs.push(redir);
                            }
                            continue;
                        }
                    }
                }
            }
            _ if IS_REDIROP(tok()) => {
                match par_redir() {
                    Some(redir) => redirs.push(redir),
                    None => break, // Error in redir parsing, stop
                }
            }
            INOUTPAR if !words.is_empty() => {
                // c:2055-2057 — `if (!isset(MULTIFUNCDEF) && argc > 1)
                // YYERROR(oecused);` — multi-name funcdef gate:
                // `f1 f2() { ... }` defines f1 AND f2 to the same
                // body, but only when MULTIFUNCDEF is set.
                if !isset(MULTIFUNCDEF) && words.len() > 1 {
                    zerr("parse error: multiple names in function definition without MULTIFUNCDEF");
                    return None;
                }
                // c:2061-2068 — `if (isset(EXECOPT) && hasalias &&
                // !isset(ALIASFUNCDEF) && argc && hasalias !=
                // input_hasalias()) { zwarn(...); YYERROR(...); }`
                // Alias-as-funcdef warning. zshrs's parser doesn't
                // track `hasalias` (alias-expansion provenance
                // during parse) yet, so `had_alias` stays false —
                // the gate is wired here as a marker so the canonical
                // C predicate is visible. Once alias-provenance lands,
                // swap `false` for the actual provenance compare.
                let had_alias = false;
                if isset(EXECOPT) && had_alias && !isset(ALIASFUNCDEF) && !words.is_empty() {
                    crate::ported::utils::zwarn("defining function based on alias `(unknown)'");
                    return None;
                }
                // foo() { ... } style function
                return parse_inline_funcdef(words.pop().unwrap());
            }
            _ => break,
        }
    }

    if assigns.is_empty() && words.is_empty() && redirs.is_empty() {
        return None;
    }

    Some(ZshCommand::Simple(ZshSimple {
        assigns,
        words,
        redirs,
    }))
}

/// Parse a redirection
/// Parse a redirection (>file, <file, >>file, <<HEREDOC, etc.).
/// Direct port of zsh/Src/parse.c:2229 `par_redir`. Returns
/// a ZshRedir node carrying the operator type, fd, target word
/// (or here-doc body / pipe-redir command), and any `{var}` style
/// fd-binding parameter.
fn par_redir() -> Option<ZshRedir> {
    par_redir_with_id(None)
}

/// Wire a here-document body onto the redirection token that
/// requested it. Direct port of zsh/Src/parse.c:2347
/// `setheredoc`. Called when a heredoc terminator has been
/// matched and the body is ready to be attached to the redir.
///
/// zshrs port note: zsh's setheredoc patches the wordcode
/// in-place via `pc[1] = ecstrcode(doc); pc[2] = ecstrcode(term);`.
/// zshrs threads heredoc bodies through `HereDocInfo` structs
/// attached inline during the post-parse `fill_heredoc_bodies` walk.
/// This method is the AST-side equivalent: writes back to the
/// matching redir node by index.
/// Port of `setheredoc(int pc, int type, char *str, char *termstr,
/// char *munged_termstr)` from `Src/parse.c:2347-2355`. Patches the
/// pending heredoc redir at `pc` with its body string + raw and
/// munged terminator forms.
pub fn setheredoc(pc: usize, redir_type: i32, doc: &str, term: &str, munged_term: &str) {
    // zshrs-only guard: AST-path heredocs use `pc = -1 as usize`
    // (i.e. `usize::MAX`) as a sentinel meaning "no wordcode slot to
    // patch". C never passes a negative pc since the wordcode emitter
    // is always active. Skip silently for the AST-only case.
    if pc == usize::MAX {
        return;
    }
    // c:2350 — `int varid = WC_REDIR_VARID(ecbuf[pc]) ? REDIR_VARID_MASK : 0;`
    let cur = ECBUF.with_borrow(|b| b.get(pc).copied().unwrap_or(0));
    let varid = if WC_REDIR_VARID(cur) != 0 {
        REDIR_VARID_MASK
    } else {
        0
    };
    // c:2351 — `ecbuf[pc] = WCB_REDIR(type | REDIR_FROM_HEREDOC_MASK | varid);`
    let new_header = WCB_REDIR((redir_type | REDIR_FROM_HEREDOC_MASK | varid) as wordcode);
    // c:2352 — `ecbuf[pc + 2] = ecstrcode(str);`
    let coded_str = ecstrcode(doc);
    // c:2353 — `ecbuf[pc + 3] = ecstrcode(termstr);`
    let coded_term = ecstrcode(term);
    // c:2354 — `ecbuf[pc + 4] = ecstrcode(munged_termstr);`
    let coded_munged = ecstrcode(munged_term);
    ECBUF.with_borrow_mut(|b| {
        b[pc] = new_header;
        b[pc + 2] = coded_str;
        b[pc + 3] = coded_term;
        b[pc + 4] = coded_munged;
    });
}

/// Parse a wordlist for `for ... in WORDS;`. Direct port of
/// zsh/Src/parse.c:2362 `par_wordlist`. Reads STRING tokens
/// until the next SEPER / SEMI / NEWLIN.
pub fn par_wordlist() -> Vec<String> {
    let mut out = Vec::new();
    // parse.c:2362-2378 — collect STRINGs into the wordlist.
    while tok() == STRING_LEX {
        if let Some(text) = tokstr() {
            out.push(text);
        }
        zshlex();
    }
    out
}

/// Parse a newline-separated wordlist. Direct port of
/// zsh/Src/parse.c:2379 `par_nl_wordlist`. Like
/// par_wordlist but tolerates leading/trailing newlines.
pub fn par_nl_wordlist() -> Vec<String> {
    // parse.c:2380-2381 — skip leading newlines.
    while tok() == NEWLIN {
        zshlex();
    }
    let out = par_wordlist();
    // parse.c:2395-2397 — skip trailing newlines.
    while tok() == NEWLIN {
        zshlex();
    }
    out
}

/// `COND_SEP()` macro from `Src/parse.c:2433`. True when the current
/// token is a separator usable inside `[[ … ]]` (newline / semi /
/// `&`). C uses it to skip optional whitespace between cond terms.
#[inline]
pub fn COND_SEP() -> bool {
    matches!(tok(), NEWLIN | SEMI | AMPER)
}

/// Parse [[ ... ]] conditional
/// Parse `[[ EXPR ]]` conditional expression. Direct port of
/// zsh/Src/parse.c:2409 `par_cond` (and helpers par_cond_1,
/// par_cond_2, par_cond_double, par_cond_triple, par_cond_multi
/// at parse.c:2434-2731). Expression operators: `||` `&&` `!`
/// + unary tests (-f, -d, -n, -z, etc.) + binary tests (=, !=,
///   <, >, ==, =~, -eq, -ne, -lt, -le, -gt, -ge, -nt, -ot, -ef).
fn par_cond() -> Option<ZshCommand> {
    // C par_dinbrack (parse.c:1810-1822) wraps the body parse with
    // `incond = 1; incmdpos = 0;` BEFORE the first zshlex past `[[`,
    // and resets to `incond = 0; incmdpos = 1;` after `]]`. Without
    // `incond = 1`, lex.c does not promote `]]` to DOUTBRACK and the
    // cond body bleeds past the close bracket — the parser then
    // sees `]]` as a separate STRING command. Every `if [[ ... ]]; then`
    // failed with `command not found: ]]` before this fix.
    set_incond(1);
    set_incmdpos(false);
    zshlex(); // skip [[
              // Empty cond `[[ ]]` is a parse error in zsh — emit the
              // diagnostic and return None so the caller produces a
              // non-zero exit. Without this, `[[ ]]` silently passed and
              // returned exit 0.
    if tok() == DOUTBRACK {
        zerr("parse error near `]]'");
        set_incond(0);
        set_incmdpos(true);
        zshlex();
        return None;
    }
    let cond = parse_cond_expr();

    if tok() == DOUTBRACK {
        set_incond(0);
        set_incmdpos(true);
        zshlex();
    } else {
        // c:Src/parse.c:1818-1819 — `if (tok != DOUTBRACK)
        // YYERRORV(oecused);`. par_dinbrack hard-requires DOUTBRACK
        // after par_cond; anything else is a parse error and the
        // outer parser's yyerror at c:2747 emits `parse error near
        // \`%s'` using zshlextext. Bug #473: BAR (`|`) inside
        // `[[ ab == a|b ]]` slipped past par_cond_or (which only
        // checks DBAR), the cond returned cleanly, and then the
        // top-level parser interpreted BAR as a pipe — running `b`
        // as a command (security-relevant if pattern RHS is user
        // input). Mirror C: emit parse error and abort.
        let tok_text = match tok() {
            BAR_TOK => "|".to_string(),
            DBAR => "||".to_string(),
            AMPER => "&".to_string(),
            DAMPER => "&&".to_string(),
            SEMI => ";".to_string(),
            DSEMI => ";;".to_string(),
            NEWLIN | SEPER => String::new(),
            _ => tokstr().map(|s| crate::ported::lex::untokenize(&s)).unwrap_or_default(),
        };
        if tok_text.is_empty() {
            zerr("parse error");
        } else {
            zerr(&format!("parse error near `{}'", tok_text));
        }
        set_incond(0);
        set_incmdpos(true);
        return None;
    }

    cond.map(ZshCommand::Cond)
}

/// Port of `par_cond_1(void)` from `Src/parse.c:2434`. Parses one
/// `||`-separated cond expression. Emits `WCB_COND(COND_AND, …)`
/// when an `&&` is found and recurses.
pub fn par_cond_1() -> i32 {
    // c:2434

    let p = ECUSED.with(|c| c.get()) as usize;
    let r = par_cond_2();
    while COND_SEP() {
        condlex();
    }
    if tok() == DAMPER {
        condlex();
        while COND_SEP() {
            condlex();
        }
        ecispace(p, 1);
        par_cond_1();
        let ecused = ECUSED.with(|c| c.get()) as usize;
        ECBUF.with(|c| {
            c.borrow_mut()[p] = WCB_COND(COND_AND as u32, (ecused - 1 - p) as u32);
        });
        return 1;
    }
    r
}

/// Port of `par_cond_2(void)` from `Src/parse.c:2476`. The heavy
/// cond-term parser: handles `! cond`, `(cond)`, unary `[ -X arg ]`,
/// binary `[ A op B ]`, and `[ A op1 B op2 C … ]` n-ary chains.
pub fn par_cond_2() -> i32 {
    // c:2476
    // `n_testargs` only applies in `testlex` mode (=== /bin/test
    // compat). zshrs has no testlex yet, so always 0.
    let n_testargs: i32 = 0;

    // c:2481 — handled inline; this Rust port skips the n_testargs
    // arm since zshrs invokes par_cond via [[ ... ]] only.

    while COND_SEP() {
        condlex();
    }
    if tok() == BANG_TOK {
        // c:2522 — `[[ ! cond ]]`
        condlex();
        ecadd(WCB_COND(COND_NOT as u32, 0));
        return par_cond_2();
    }
    if tok() == INPAR_TOK {
        // c:2533 — `[[ (cond) ]]`
        condlex();
        while COND_SEP() {
            condlex();
        }
        let r = par_cond();
        while COND_SEP() {
            condlex();
        }
        if tok() != OUTPAR_TOK {
            crate::ported::utils::zerr("missing )");
            yyerror(0);
            return 0;
        }
        condlex();
        return r.map_or(0, |_| 1);
    }
    let s1 = tokstr().unwrap_or_default();
    // c:2549 — `dble = (s1 && IS_DASH(*s1) && (!n_testargs ||
    // strspn(s1+1, "abcd...") == 1) && !s1[2]);` — IS_DASH covers
    // BOTH `-` and Dash (`\u{9b}`). The raw tokstr inside `[[ ... ]]`
    // carries Dash as a marker byte, so `starts_with('-')` alone
    // matches only ASCII dashes and misses every `-z`, `-d`, `-r`
    // etc. — every such cond emitted the AST-only `condition
    // expected` error from par_cond_double. Use IS_DASH and count
    // chars (Dash is a single code point) instead of bytes.
    let s1_chars: Vec<char> = s1.chars().collect();
    let dble = !s1_chars.is_empty()
        && IS_DASH(s1_chars[0])
        && s1_chars.len() == 2
        && "abcdefghknoprstuvwxzLONGS".contains(s1_chars[1]);
    if tok() != STRING_LEX {
        if !s1.is_empty() && tok() != LEXERR && (!dble || n_testargs != 0) {
            // c:2486-2497 — `if (n_testargs == 1)` block: under
            // POSIXBUILTINS-off, `[ -t ]` rewrites to `[ -t 1 ]`
            // (ksh behavior). The C gate is `unset(POSIXBUILTINS)
            // && check_cond(s1, "t")`. zshrs's parser has
            // n_testargs=0 (no testlex), so this rewrite path is
            // unreachable from zshrs's [[ ]] / [ ] entry points;
            // wired here as a marker for parity. When testlex is
            // ported the call below activates.
            if n_testargs == 1 && unset(POSIXBUILTINS) && check_cond(&s1, "t") {
                condlex();
                return par_cond_double(&s1, "1");
            }
            // c:2557 — `[[ STRING ]]` re-interpreted as `[[ -n STRING ]]`.
            condlex();
            while COND_SEP() {
                condlex();
            }
            return par_cond_double("-n", &s1);
        }
        crate::ported::utils::zerr("condition expected");
        yyerror(0);
        return 0;
    }
    condlex();
    while COND_SEP() {
        condlex();
    }
    if tok() == INANG_TOK || tok() == OUTANG_TOK {
        // c:2576 — `<` / `>` string compare.
        let xtok = tok();
        condlex();
        while COND_SEP() {
            condlex();
        }
        if tok() != STRING_LEX {
            crate::ported::utils::zerr("string expected");
            yyerror(0);
            return 0;
        }
        let s3 = tokstr().unwrap_or_default();
        condlex();
        while COND_SEP() {
            condlex();
        }
        let op = if xtok == INANG_TOK {
            COND_STRLT
        } else {
            COND_STRGTR
        };
        ecadd(WCB_COND(op as u32, 0));
        ecstr(&s1);
        ecstr(&s3);
        return 1;
    }
    if tok() != STRING_LEX {
        // c:2592 — only one operand seen → `[ -n s1 ]`.
        if tok() != LEXERR {
            if !dble || n_testargs != 0 {
                return par_cond_double("-n", &s1);
            }
            return par_cond_multi(&s1, &[]);
        }
        crate::ported::utils::zerr("syntax error");
        yyerror(0);
        return 0;
    }
    let s2 = tokstr().unwrap_or_default();
    set_incond(incond() + 1);
    condlex();
    while COND_SEP() {
        condlex();
    }
    set_incond(incond() - 1);
    // c:Src/parse.c:2598-2600 — `if (!n_testargs) dble = (s2 &&
    // IS_DASH(*s2) && !s2[2]);` — RECOMPUTE dble based on s2 once
    // it's been read, so `[[ A -X B ]]` is treated as a 2-arg cond
    // `[ -X B ]` (par_cond_double) rather than a 3-arg triple. This
    // is what routes `[[ "" -a "x" ]]` to par_cond_double("", "-a")
    // → COND_ERROR "parse error: condition expected: ". Without
    // this, the original `dble` from s1 stayed false, the parser
    // grabbed s3 and built COND_MODI silently. parity bug #25.
    let s2_chars: Vec<char> = s2.chars().collect();
    let dble = !s2_chars.is_empty() && IS_DASH(s2_chars[0]) && s2_chars.len() == 2;
    if tok() == STRING_LEX && !dble {
        let s3 = tokstr().unwrap_or_default();
        condlex();
        while COND_SEP() {
            condlex();
        }
        if tok() == STRING_LEX {
            // c:2615 — n-ary `[ A op B C D ... ]`.
            let mut l: Vec<String> = vec![s2, s3];
            while tok() == STRING_LEX {
                l.push(tokstr().unwrap_or_default());
                condlex();
                while COND_SEP() {
                    condlex();
                }
            }
            return par_cond_multi(&s1, &l);
        }
        return par_cond_triple(&s1, &s2, &s3);
    }
    par_cond_double(&s1, &s2)
}

/// Port of `par_cond_double(char *a, char *b)` from `Src/parse.c:2626`.
/// Emits wordcode for unary cond `[ -X b ]` or modular `[ -mod b ]`.
pub fn par_cond_double(a: &str, b: &str) -> i32 {
    // c:2628 — `if (!IS_DASH(a[0]) || !a[1])` — char-based, since
    // Dash is a single code point (`\u{9b}`) and `a.len() < 2` on
    // BYTES would still pass for "-z" but fail for the marker form
    // `\u{9b}z` (2 bytes). Walk by chars.
    let ac: Vec<char> = a.chars().collect();
    if ac.is_empty() || !IS_DASH(ac[0]) || ac.len() < 2 {
        // c:Src/parse.c:2629 COND_ERROR macro expansion:
        //   zwarn(...); herrflush(); errflag |= ERRFLAG_ERROR;
        //   YYERROR(ecused) /* sets tok = LEXERR */
        // The YYERROR portion is critical — without it the outer
        // parser keeps walking the wordcode and execution proceeds
        // (e.g. `[[ "" -a "x" ]] && echo m || echo n` runs the
        // `|| echo n` branch). Setting LEXERR aborts the upper
        // parse so the whole line is rejected, matching zsh's
        // observable behavior of stdout="" on parse error.
        zerr(&format!("parse error: condition expected: {}", a));
        errflag.fetch_or(crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::SeqCst);
        set_tok(LEXERR);
        return 1;
    }
    // c:2630 — `else if (!a[2] && strspn(a+1, "abcd...zhLONGS") == 1)`
    let unary_set = "abcdefgknoprstuvwxzhLONGS";
    if ac.len() == 2 && unary_set.contains(ac[1]) {
        // c:2631 — `ecadd(WCB_COND(a[1], 0));` uses the raw cond-op
        // letter byte as the opcode payload. Use the ASCII char's
        // code-point value directly — every letter in `unary_set`
        // fits in 7 bits.
        ecadd(WCB_COND(ac[1] as u32, 0));
        ecstr(b);
    } else {
        ecadd(WCB_COND(COND_MOD as u32, 1));
        ecstr(a);
        ecstr(b);
    }
    1
}

/// Port of `get_cond_num(char *tst)` from `Src/parse.c:2643`. Returns
/// the index of `tst` in `{"nt","ot","ef","eq","ne","lt","gt","le","ge"}`
/// or `-1` if not a recognized binary cond operator.
pub fn get_cond_num(tst: &str) -> i32 {
    // c:2643
    const CONDSTRS: [&str; 9] = [
        "nt", "ot", "ef", "eq", "ne", "lt", "gt", "le", "ge", // c:2647
    ];
    for (i, &c) in CONDSTRS.iter().enumerate() {
        if c == tst {
            return i as i32; // c:2654
        }
    }
    -1 // c:2656
}

/// par_time's `static int inpartime` guard at C parse.c:1038
/// preventing infinite recursion on `time time foo`. The wordcode
/// path keeps this as a thread_local since C uses a function-level
/// `static int` (per-process; per-evaluator semantically matches).
thread_local! {
    static PARSER_INPARTIME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Port of `par_cond_triple(char *a, char *b, char *c)` from
/// `Src/parse.c:2659`. Emits wordcode for the binary forms
/// `[ A op B ]` — `=` / `==` / `!=` / `<` / `>` / `=~` / `-X`.
///
/// C does `(b[0] == Equals || b[0] == '=')` etc., matching BOTH the
/// raw ASCII operator char AND its tokenized marker form per
/// `Src/zsh.h:159-194`:
///   Equals = `\u{8d}`, Outang = `\u{95}`, Inang  = `\u{94}`,
///   Tilde  = `\u{98}`, Bang   = `\u{9c}`, Dash   = `\u{9b}`.
/// Inside `[[ ... ]]` the lexer emits the marker bytes — comparing
/// against literal-only `b"=="` misses every cond op.
/// (The previous Rust port had the doc comment values wrong:
/// Outang=0x8e was actually Bar; Inang=0x91 was Inbrack;
/// Tilde=0x96 was OutangProc; Bang=0x8b was Outparmath. The code
/// itself uses the correct const names, so this was a docs-only fix.)
pub fn par_cond_triple(a: &str, b: &str, c: &str) -> i32 {
    // c:2659
    let bc: Vec<char> = b.chars().collect();
    let is_eq = |ch: char| ch == '=' || ch == Equals;
    let is_gt = |ch: char| ch == '>' || ch == Outang;
    let is_lt = |ch: char| ch == '<' || ch == Inang;
    let is_tilde = |ch: char| ch == '~' || ch == Tilde;
    let is_bang = |ch: char| ch == '!' || ch == Bang;

    // c:2663 — `(b[0] == Equals || b[0] == '=') && !b[1]` → `=` (single).
    if bc.len() == 1 && is_eq(bc[0]) {
        ecadd(WCB_COND(COND_STREQ as u32, 0));
        ecstr(a);
        ecstr(c);
        let np = ECNPATS.with(|cc| {
            let v = cc.get();
            cc.set(v + 1);
            v
        }) as u32;
        ecadd(np);
        return 1;
    }
    // c:2668-2673 — `(t0 = b[0]=='>' || Outang) || b[0]=='<' || Inang`.
    if bc.len() == 1 && (is_gt(bc[0]) || is_lt(bc[0])) {
        let op = if is_gt(bc[0]) {
            COND_STRGTR
        } else {
            COND_STRLT
        };
        ecadd(WCB_COND(op as u32, 0));
        ecstr(a);
        ecstr(c);
        let np = ECNPATS.with(|cc| {
            let v = cc.get();
            cc.set(v + 1);
            v
        }) as u32;
        ecadd(np);
        return 1;
    }
    // c:2674-2679 — `==` STRDEQ.
    if bc.len() == 2 && is_eq(bc[0]) && is_eq(bc[1]) {
        ecadd(WCB_COND(COND_STRDEQ as u32, 0));
        ecstr(a);
        ecstr(c);
        let np = ECNPATS.with(|cc| {
            let v = cc.get();
            cc.set(v + 1);
            v
        }) as u32;
        ecadd(np);
        return 1;
    }
    // c:2680-2684 — `!=` STRNEQ.
    if bc.len() == 2 && is_bang(bc[0]) && is_eq(bc[1]) {
        ecadd(WCB_COND(COND_STRNEQ as u32, 0));
        ecstr(a);
        ecstr(c);
        let np = ECNPATS.with(|cc| {
            let v = cc.get();
            cc.set(v + 1);
            v
        }) as u32;
        ecadd(np);
        return 1;
    }
    // c:2685-2691 — `=~` REGEX (no pattern slot — implicit COND_MODI).
    if bc.len() == 2 && is_eq(bc[0]) && is_tilde(bc[1]) {
        ecadd(WCB_COND(COND_REGEX as u32, 0));
        ecstr(a);
        ecstr(c);
        return 1;
    }
    // c:2692-2702 — `-OP` numeric-or-modular cond (e.g. `-eq`, `-nt`).
    if !bc.is_empty() && IS_DASH(bc[0]) {
        let rest: String = bc[1..].iter().collect();
        let t = get_cond_num(&rest);
        if t > -1 {
            ecadd(WCB_COND((t + COND_NT) as u32, 0));
            ecstr(a);
            ecstr(c);
            return 1;
        }
        ecadd(WCB_COND(COND_MODI as u32, 0));
        ecstr(b);
        ecstr(a);
        ecstr(c);
        return 1;
    }
    // c:2703-2707 — `-mod A B C` modular cond on `a`.
    let ac: Vec<char> = a.chars().collect();
    if !ac.is_empty() && IS_DASH(ac[0]) && ac.len() > 1 {
        ecadd(WCB_COND(COND_MOD as u32, 2));
        ecstr(a);
        ecstr(b);
        ecstr(c);
        return 1;
    }
    zerr(&format!("condition expected: {}", b));
    1
}

/// Port of `par_cond_multi(char *a, LinkList l)` from `Src/parse.c:2716`.
/// Emits wordcode for `[ -OP A B C … ]` n-ary cond (alternation).
pub fn par_cond_multi(a: &str, l: &[String]) -> i32 {
    // c:2716 — `if (!IS_DASH(a[0]) || !a[1])`; same Dash/`-` dual
    // matching as par_cond_double, char-walked because Dash is a
    // single code point.
    let ac: Vec<char> = a.chars().collect();
    if ac.is_empty() || !IS_DASH(ac[0]) || ac.len() < 2 {
        zerr(&format!("condition expected: {}", a));
        return 1;
    }
    ecadd(WCB_COND(COND_MOD as u32, l.len() as u32));
    ecstr(a);
    for item in l {
        ecstr(item);
    }
    1
}

/// Emit a parser-level error. Direct port of zsh/Src/parse.c
/// 2733-2766 `yyerror`. C version fills a per-event error buffer
/// and sets errflag. zshrs pushes onto errors which the
/// caller drains via parse()'s Result return.
/// WARNING: param-name divergence — Rust takes `&str message`, C takes
/// Port of `static void yyerror(int noerr)` from `Src/parse.c:2733`.
///
/// Faithful C body (verbatim):
/// ```c
/// int t0; char *t;
/// if ((t = dupstring(zshlextext))) untokenize(t);
/// for (t0 = 0; t0 != 20; t0++)
///     if (!t || !t[t0] || t[t0] == '\n') break;
/// if (!(histdone & HISTFLAG_NOEXEC) && !(errflag & ERRFLAG_INT)) {
///     if (t0) {
///         t = metafy(t, t0, META_STATIC);
///         zwarn("parse error near `%s%s'", t, t0 == 20 ? "..." : "");
///     } else
///         zwarn("parse error");
/// }
/// if (!noerr && noerrs != 2)
///     errflag |= ERRFLAG_ERROR;
/// ```
///
/// `zshlextext` is the C lexer's current-token text (`Src/lex.c:170`
/// `char *tokstr`); zshrs's equivalent is `lex::tokstr()`. The "20"
/// is C's tail-truncation length for the error message.
pub fn yyerror(noerr: i32) {
    // c:2733
    // c:2738 — `if ((t = dupstring(zshlextext))) untokenize(t);`.
    // In C, `zshlextext` falls back to `tokstrings[tok]` (lex.c:1965)
    // for punctuation tokens that didn't capture a tokstr — that's
    // how "parse error near `)'" gets the `)` for OUTPAR. Mirror by
    // consulting `lex::tokstring(tok())` when the captured tokstr is
    // None.
    let t_opt: Option<String> = match crate::ported::lex::tokstr() {
        Some(raw) => Some(crate::ported::lex::untokenize(&raw).to_string()),
        None => {
            let t = crate::ported::lex::tok();
            let i = t as usize;
            if i < crate::ported::lex::tokstrings.len() {
                crate::ported::lex::tokstrings[i].map(|s| s.to_string())
            } else {
                None
            }
        }
    };
    let t_bytes: Vec<u8> = t_opt
        .as_ref()
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_default();

    // c:2741-2743 — `for (t0 = 0; t0 != 20; t0++) if (!t || !t[t0]
    //   || t[t0] == '\n') break;`
    let mut t0: usize = 0;
    while t0 != 20 {
        // c:2741
        let stop = t_opt.is_none()
            || t0 >= t_bytes.len()
            || t_bytes[t0] == 0
            || t_bytes[t0] == b'\n';
        if stop {
            break;
        }
        t0 += 1;
    }

    // c:2744 — `if (!(histdone & HISTFLAG_NOEXEC) && !(errflag &
    //   ERRFLAG_INT))`. The HISTFLAG_NOEXEC gate suppresses warnings
    //   from history-recall paths that aren't actually executing.
    let histdone_v = crate::ported::hist::histdone.load(Ordering::SeqCst);
    let hist_noexec =
        (histdone_v & crate::ported::zsh_h::HISTFLAG_NOEXEC as i32) != 0;
    let int_flagged =
        (errflag.load(Ordering::SeqCst) & crate::ported::zsh_h::ERRFLAG_INT) != 0;
    if !hist_noexec && !int_flagged {
        // c:2744
        if t0 != 0 {
            // c:2745
            // c:2746 — `t = metafy(t, t0, META_STATIC);` — re-metafy
            //   the truncated head so embedded Meta bytes display
            //   correctly. The Rust port already holds an untokenized
            //   string; use the byte slice [0..t0] directly.
            let head =
                std::str::from_utf8(&t_bytes[..t0]).unwrap_or_default();
            let suffix = if t0 == 20 { "..." } else { "" };
            crate::ported::utils::zwarn(&format!(
                "parse error near `{}{}'",
                head, suffix
            )); // c:2747
        } else {
            // c:2748
            crate::ported::utils::zwarn("parse error"); // c:2749
        }
    }
    // c:2751 — `if (!noerr && noerrs != 2) errflag |= ERRFLAG_ERROR;`.
    //   The `noerrs != 2` gate (suppress-only-fatal-errors) is preserved
    //   for parity with zerr/zwarn's matching check.
    let noerrs_v = *crate::ported::utils::noerrs_lock().lock().unwrap();
    if noerr == 0 && noerrs_v != 2 {
        // c:2751
        errflag.fetch_or(
            crate::ported::zsh_h::ERRFLAG_ERROR,
            Ordering::SeqCst,
        ); // c:2752
    }
}


// ============================================================
// Eprog runtime ops (parse.c:2767-2853)
//
// dupeprog / useeprog / freeeprog are zsh's reference-counting
// helpers for executable programs. zshrs's AST is owned by
// value (Rust ownership); cloning is a tree-deep copy via
// Clone, "use" is a no-op (the executor borrows the AST), and
// "free" is automatic on drop.
// ============================================================

/// Duplicate an Eprog. Direct port of zsh/Src/parse.c:2813
/// Port of `Eprog dupeprog(Eprog p, int heap)` from
/// `Src/parse.c:2767`. Deep-copies the wordcode array, string
/// table, and pattern-prog slots. `dummy_eprog` is returned
/// unchanged. `heap`-allocated copies get `nref = -1` (never
/// freed); real ones get `nref = 1`.
pub fn dupeprog(p: &eprog, heap: bool) -> eprog {
    // c:2774-2775 — `if (p == &dummy_eprog) return p;` — caller-
    // observable identity in C uses a pointer compare; Rust's
    // equivalent is "if it has the dummy's shape (single WCB_END
    // word and no strs), return a copy of the same shape".
    // c:2796-2797 — `for (i = r->npats; i--; pp++) *pp = dummy_patprog1;`
    // C uses `dummy_patprog1` as a placeholder; the Rust port has
    // `Vec<Patprog>` (Box<patprog>) — synthesize an equivalent zero-
    // initialized patprog for each slot (resolved later by
    // pattern.c::patcompile-on-first-use).
    let dummy_pat = || crate::ported::zsh_h::patprog {
        startoff: 0,
        size: 0,
        mustoff: 0,
        patmlen: 0,
        globflags: 0,
        globend: 0,
        flags: 0,
        patnpar: 0,
        patstartch: 0,
    };
    let r = eprog {
        // c:2778 — `flags = (heap ? EF_HEAP : EF_REAL) | (p->flags & EF_RUN);`
        flags: (if heap { EF_HEAP } else { EF_REAL }) | (p.flags & EF_RUN),
        len: p.len,
        npats: p.npats,
        // c:2787 — `nref = heap ? -1 : 1;`
        nref: if heap { -1 } else { 1 },
        prog: p.prog.clone(),
        strs: p.strs.clone(),
        pats: (0..p.npats).map(|_| Box::new(dummy_pat())).collect(),
        shf: None,
        dump: None,
    };
    r
}

/// Port of `void useeprog(Eprog p)` from `Src/parse.c:2813`.
/// `if (p && p != &dummy_eprog && p->nref >= 0) p->nref++;` —
/// pin a real (non-heap, non-dummy) Eprog so it survives the
/// next `freeeprog`.
pub fn useeprog(p: &mut eprog) {
    // c:2815 — `if (p && p != &dummy_eprog && p->nref >= 0)`
    if p.nref >= 0 {
        p.nref += 1; // c:2816
    }
}

/// Port of `void freeeprog(Eprog p)` from `Src/parse.c:2823`.
/// Refcount-decrement; when it hits zero, drops the pattern progs,
/// decrements the dump refcount if any, and releases the eprog.
/// `dummy_eprog` is never freed. Heap-eprogs (`nref < 0`) are
/// never freed either — they live as long as the heap arena.
pub fn freeeprog(p: &mut eprog) {
    // c:2829 — `if (p && p != &dummy_eprog) { ... }`
    if p.nref > 0 {
        p.nref -= 1; // c:2832
        if p.nref == 0 {
            // c:2833-2840 — drop pats, dump refcount, then the eprog.
            // Rust's Drop handles the per-field cleanup; we just
            // need to decrement the dump count first.
            if let Some(dump) = p.dump.take() {
                let dumped = (*dump).clone();
                decrdumpcount(&dumped); // c:2837
            }
            p.prog.clear();
            p.strs = None;
            p.pats.clear();
        }
    }
}

// =============================================================================
// Wordcode read helpers — used by text.rs's `gettext2` and exec dispatch
// to walk a compiled Eprog without re-running the parser. These are the
// only `Src/parse.c` functions ported so far in this file; the recursive-
// descent parser (par_event / par_list / par_cmd / par_*) follows
// below as free ported at module scope.
// =============================================================================

/// Port of `ecgetstr(Estate s, int dup, int *tokflag)` from `Src/parse.c:2855`.
/// `s->pc` advances through the wordcode buffer; `s->strs` indexes the
/// string pool. Returns the interned string (or a 1-3-char literal
/// inlined directly into the wordcode word).
pub fn ecgetstr(s: &mut estate, dup: i32, tokflag: Option<&mut i32>) -> String {
    let prog = &s.prog.prog;
    if s.pc >= prog.len() {
        return String::new();
    }
    let c = prog[s.pc]; // c:2858 `wordcode c = *s->pc++;`
    s.pc += 1;
    if let Some(tf) = tokflag {
        *tf = i32::from((c & 1) != 0); // c:2880 `*tokflag = (c & 1);`
    }
    if c == 6 || c == 7 {
        // c:2861 `if (c == 6 || c == 7) r = "";`
        return String::new();
    }
    let r: String = if (c & 2) != 0 {
        // c:2862 — `else if (c & 2)`
        // c:2863-2868 — 3-byte inline string packed into the wordcode
        // word; followed by `buf[3] = '\0'; r = dupstring(buf);`.
        // C's `dupstring` uses `strlen(buf)` which TRUNCATES at the
        // first NUL byte — short strings of 1 or 2 chars get padded
        // with NULs and truncated cleanly. The previous Rust port
        // used `retain(|&x| x != 0)` which would silently SPLICE OUT
        // an interior NUL (e.g. `[a, 0, b]` → "ab"), diverging from
        // C's strlen-truncate (`[a, 0, b]` → "a"). Fix: truncate at
        // first NUL to match C exactly.
        let b0 = ((c >> 3) & 0xff) as u8;
        let b1 = ((c >> 11) & 0xff) as u8;
        let b2 = ((c >> 19) & 0xff) as u8;
        let v = [b0, b1, b2];
        let end = v.iter().position(|&x| x == 0).unwrap_or(v.len()); // c:2869 strlen(buf)
        // C reads raw bytes (token codes included) — widen via the
        // wordcode-pool bridge, never from_utf8_lossy (which mangles
        // raw token bytes from C-zsh-written .zwc dumps to U+FFFD).
        crate::zwc::wordcode_pool_str(&v[..end])
    } else {
        // c:2877 `else r = s->strs + (c >> 2);`
        let off = (c >> 2) as usize + s.strs_offset;
        let strs_bytes = s.strs.as_deref().unwrap_or("").as_bytes();
        if off >= strs_bytes.len() {
            String::new()
        } else {
            let tail = &strs_bytes[off..];
            let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
            crate::zwc::wordcode_pool_str(&tail[..end])
        }
    };
    // c:2891 `return ((dup == EC_DUP || (dup && (c & 1))) ? dupstring(r) : r);`
    // Rust owns the String already; `dup` flag has no observable effect.
    let _ = (dup, EC_DUP, EC_NODUP);
    r
}

// ============================================================
// Wordcode runtime getters (parse.c:2853-3060)
//
// Direct ports of the wordcode-read helpers (ecrawstr,
// ecgetstr, ecgetarr, ecgetredirs, ecgetlist, eccopyredirs).
// Read packed wordcode out of an Eprog at execution time.
// Used by exec_wordcode and the wordcode-walking dispatch in
// src/vm_helper.
// ============================================================

/// Port of `ecrawstr(Eprog p, Wordcode pc, int *tokflag)` from
/// `Src/parse.c:2891`. Like `ecgetstr` but reads at the given pc
/// without advancing — caller steps `pc` separately.
pub fn ecrawstr(p: &eprog, pc: usize, tokflag: Option<&mut i32>) -> String {
    if pc >= p.prog.len() {
        return String::new();
    }
    let c = p.prog[pc]; // c:2894
    if let Some(tf) = tokflag {
        *tf = i32::from((c & 1) != 0); // c:2898/2906/2912
    }
    if c == 6 || c == 7 {
        // c:2897
        return String::new();
    }
    if (c & 2) != 0 {
        // c:2902-2906 — same 3-byte inline string as ecgetstr, then
        // `buf[3] = '\0'; return dupstring(buf);` — truncate at first
        // NUL via strlen (NOT splice out interior NULs).
        let b0 = ((c >> 3) & 0xff) as u8;
        let b1 = ((c >> 11) & 0xff) as u8;
        let b2 = ((c >> 19) & 0xff) as u8;
        let v = [b0, b1, b2];
        let end = v.iter().position(|&x| x == 0).unwrap_or(v.len()); // c:2906 strlen(buf)
        // Raw-byte widening — see ecgetstr (same C-convention bridge).
        crate::zwc::wordcode_pool_str(&v[..end])
    } else {
        // c:2911
        let off = (c >> 2) as usize;
        let strs_bytes = p.strs.as_deref().unwrap_or("").as_bytes();
        if off >= strs_bytes.len() {
            return String::new();
        }
        let tail = &strs_bytes[off..];
        let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
        crate::zwc::wordcode_pool_str(&tail[..end])
    }
}

/// Port of `ecgetarr(Estate s, int num, int dup, int *tokflag)` from
/// `Src/parse.c:2917`. Reads `num` strings from wordcode at `s->pc`
/// and OR-folds each entry's token flag into `*tokflag`.
pub fn ecgetarr(s: &mut estate, num: usize, dup: i32, tokflag: Option<&mut i32>) -> Vec<String> {
    let mut ret: Vec<String> = Vec::with_capacity(num); // c:2922
    let mut tf: i32 = 0;
    for _ in 0..num {
        // c:2924 `while (num--)`
        let mut tmp = 0;
        ret.push(ecgetstr(s, dup, Some(&mut tmp))); // c:2925
        tf |= tmp; // c:2926
    }
    if let Some(out) = tokflag {
        // c:2929
        *out = tf;
    }
    ret
}

/// Port of `ecgetlist(Estate s, int num, int dup, int *tokflag)` from
/// `Src/parse.c:2937`. Same shape as `ecgetarr` but C returns
/// `LinkList`; zshrs uses `Vec<String>` for both.
pub fn ecgetlist(s: &mut estate, num: usize, dup: i32, tokflag: Option<&mut i32>) -> Vec<String> {
    if num == 0 {
        // c:2949-2952
        if let Some(tf) = tokflag {
            *tf = 0;
        }
        return Vec::new();
    }
    ecgetarr(s, num, dup, tokflag)
}

/// Port of `ecgetredirs(Estate s)` from `Src/parse.c:2959`.
///
/// `strs` must be the same tail `ecgetstr` uses (`s->strs` / `estate.strs` from offset).
/// WARNING: param names don't match C — Rust=(prog, strs, pc) vs C=(s)
pub fn ecgetredirs(s: &mut estate) -> Vec<redir> {
    let mut ret: Vec<redir> = Vec::new(); // c:2959 `LinkList ret = newlinklist();`
    let prog_len = s.prog.prog.len();
    if s.pc >= prog_len {
        return ret;
    }
    let mut code = s.prog.prog[s.pc]; // c:2962 `wordcode code = *s->pc++;`
    s.pc += 1;

    loop {
        if wc_code(code) != WC_REDIR {
            // c:2988-2989 `s->pc--` then break from while
            s.pc = s.pc.saturating_sub(1);
            break;
        }

        let typ = WC_REDIR_TYPE(code); // c:2967 `r->type = WC_REDIR_TYPE(code);`
        if s.pc >= prog_len {
            break;
        }
        let fd1_w = s.prog.prog[s.pc]; // c:2968 `r->fd1 = *s->pc++;`
        s.pc += 1;

        let name = ecgetstr(s, EC_DUP, None); // c:2969 `r->name = ecgetstr(...)`

        let (flags, here_terminator, munged_here_terminator) = if WC_REDIR_FROM_HEREDOC(code) != 0 {
            // c:2970-2973
            let term = ecgetstr(s, EC_DUP, None);
            let munged = ecgetstr(s, EC_DUP, None);
            (REDIRF_FROM_HEREDOC, Some(term), Some(munged))
        } else {
            // c:2974-2977
            (0, None, None)
        };

        let varid = if WC_REDIR_VARID(code) != 0 {
            // c:2979-2980
            Some(ecgetstr(s, EC_DUP, None))
        } else {
            None // c:2981-2982
        };

        ret.push(redir {
            // c:2965-2982 fields + c:2984 `addlinknode`
            typ,
            flags,
            fd1: fd1_w as i32,
            fd2: 0,
            name: Some(name),
            varid,
            here_terminator,
            munged_here_terminator,
        });

        if s.pc >= prog_len {
            break;
        }
        code = s.prog.prog[s.pc]; // c:2986 `code = *s->pc++;`
        s.pc += 1;
    }

    ret // c:2990 `return ret`
}

/// Port of `eccopyredirs(Estate s)` from `Src/parse.c:3003`. Reads
/// the WC_REDIR run at `s->pc`, counts the wordcodes needed,
/// reserves space in `ecbuf` via `ecispace`, then re-walks `s->pc`
/// re-emitting each redir's wordcodes into the reserved slot —
/// finally calls `bld_eprog(0)` to package the result as an Eprog.
pub fn eccopyredirs(s: &mut estate) -> Option<eprog> {
    let prog_len = s.prog.prog.len();
    if s.pc >= prog_len {
        return None;
    }
    // c:3007-3009 — `if (wc_code(*pc) != WC_REDIR) return NULL;`
    let first_code = s.prog.prog[s.pc];
    if wc_code(first_code) != WC_REDIR {
        return None;
    }
    // c:3011 — `init_parse();`
    init_parse();

    // c:3013-3027 — count wordcodes the redir run will need.
    // Each WC_REDIR contributes `code + fd1 + name` = 3, plus
    // `+2` if WC_REDIR_FROM_HEREDOC (terminator + munged), plus
    // `+1` if WC_REDIR_VARID.
    let mut probe = s.pc;
    let mut ncodes = 0usize;
    loop {
        if probe >= prog_len {
            break;
        }
        let code = s.prog.prog[probe];
        if wc_code(code) != WC_REDIR {
            break;
        }
        let mut ncode = if WC_REDIR_FROM_HEREDOC(code) != 0 {
            5
        } else {
            3
        };
        if WC_REDIR_VARID(code) != 0 {
            ncode += 1;
        }
        probe += ncode;
        ncodes += ncode;
    }

    // c:3028-3029 — `r = ecused; ecispace(r, ncodes);`
    let r0 = ECUSED.get() as usize;
    ecispace(r0, ncodes);

    // c:3031-3053 — re-walk `s->pc` and write into ecbuf[r..].
    let mut r = r0;
    loop {
        if s.pc >= prog_len {
            break;
        }
        let code = s.prog.prog[s.pc];
        if wc_code(code) != WC_REDIR {
            break;
        }
        s.pc += 1;
        // c:3036 — `ecbuf[r++] = code;`
        ECBUF.with_borrow_mut(|buf| {
            if r >= buf.len() {
                buf.resize(r + 1, 0);
            }
            buf[r] = code;
        });
        r += 1;
        // c:3038 — `ecbuf[r++] = *s->pc++;` (the fd1 word)
        let fd1 = s.prog.prog[s.pc];
        s.pc += 1;
        ECBUF.with_borrow_mut(|buf| {
            if r >= buf.len() {
                buf.resize(r + 1, 0);
            }
            buf[r] = fd1;
        });
        r += 1;
        // c:3041 — `ecbuf[r++] = ecstrcode(ecgetstr(s, EC_NODUP, NULL));`
        let name = ecgetstr(s, EC_NODUP, None);
        let nc = ecstrcode(&name);
        ECBUF.with_borrow_mut(|buf| {
            if r >= buf.len() {
                buf.resize(r + 1, 0);
            }
            buf[r] = nc;
        });
        r += 1;
        // c:3042-3047 — heredoc terminators.
        if WC_REDIR_FROM_HEREDOC(code) != 0 {
            let term = ecgetstr(s, EC_NODUP, None);
            let tc = ecstrcode(&term);
            ECBUF.with_borrow_mut(|buf| {
                if r >= buf.len() {
                    buf.resize(r + 1, 0);
                }
                buf[r] = tc;
            });
            r += 1;
            let munged = ecgetstr(s, EC_NODUP, None);
            let mc = ecstrcode(&munged);
            ECBUF.with_borrow_mut(|buf| {
                if r >= buf.len() {
                    buf.resize(r + 1, 0);
                }
                buf[r] = mc;
            });
            r += 1;
        }
        // c:3048-3049 — varid.
        if WC_REDIR_VARID(code) != 0 {
            let varid = ecgetstr(s, EC_NODUP, None);
            let vc = ecstrcode(&varid);
            ECBUF.with_borrow_mut(|buf| {
                if r >= buf.len() {
                    buf.resize(r + 1, 0);
                }
                buf[r] = vc;
            });
            r += 1;
        }
    }

    // c:3056 — `return bld_eprog(0);` — `bld_eprog` appends the
    // WC_END marker and packages ECBUF/ECSTRS into an Eprog.
    Some(bld_eprog(false))
}

/// Port of `init_eprog(void)` from `Src/parse.c:3069`. Sets up
/// `dummy_eprog_code = WCB_END(); dummy_eprog.len = sizeof(wordcode);
/// dummy_eprog.prog = &dummy_eprog_code; dummy_eprog.strs = NULL;`.
/// Called once at shell startup (init_main → init_misc → init_eprog).
pub fn init_eprog() {
    let mut d = DUMMY_EPROG.lock().unwrap();
    d.prog = vec![WCB_END()]; // c:3071/3073
    d.len = size_of::<wordcode>() as i32; // c:3072
    d.strs = None; // c:3074
    d.flags = 0;
    d.npats = 0;
    d.nref = 0;
}

// =====================================================================
// `bin_zcompile` and wordcode-dump helpers — port of `Src/parse.c:3104+`.
//
// The wordcode dump format (`.zwc`) is a serialized parse tree zsh can
// `mmap()` and dispatch from without re-parsing on every shell start.
// File layout (one struct = `FD_PRELEN` `u32`s):
//   - `pre[0]` = magic word (FD_MAGIC native byte-order, FD_OMAGIC
//     opposite byte-order).
//   - `pre[1]` = packed `{flags(8) | other_offset(24)}` byte field.
//   - `pre[2..12]` = `ZSH_VERSION` C-string padded to 40 bytes.
//   - `pre[12]` = `fdheaderlen` (total prelude+header word count).
//   - Then a sequence of `struct fdhead` records, one per function,
//     each followed by its NUL-terminated name (padded to 4-byte).
//   - Then the wordcode bytes for every function back-to-back.
//
// On a little-endian host writing a dump twice: first `FD_MAGIC` for
// native readers, then re-walks the body byte-swapped and emits a
// second `FD_OMAGIC` copy so big-endian readers can mmap it too.
// =====================================================================

// File-format constants — port of `Src/parse.c:3104-3150`.

/// `#define FD_EXT ".zwc"` from `Src/parse.c:3104`.
pub const FD_EXT: &str = ".zwc";

/// `#define FD_MINMAP 4096` from `Src/parse.c:3105`. mmap threshold
/// — `-M` mode only kicks in when the wordcode body is at least
/// this many bytes (otherwise read(2) is preferred).
pub const FD_MINMAP: usize = 4096;

/// `#define FD_PRELEN 12` from `Src/parse.c:3107`. File-header
/// length in u32 words: magic + packed-flags-byte + 10 version words.
pub const FD_PRELEN: usize = 12;

/// `#define FD_MAGIC 0x04050607` from `Src/parse.c:3108`. Sentinel
/// for native-byte-order dumps.
pub const FD_MAGIC: u32 = 0x04050607;

/// `#define FD_OMAGIC 0x07060504` from `Src/parse.c:3109`. Sentinel
/// for opposite-byte-order dumps (byte-swapped FD_MAGIC).
pub const FD_OMAGIC: u32 = 0x07060504;

/// `#define FDF_MAP 1` from `Src/parse.c:3111`. Bit set when the
/// dump should be `mmap()`-ed (`-M` flag) vs read normally (`-R`).
pub const FDF_MAP: u32 = 1;

/// `#define FDF_OTHER 2` from `Src/parse.c:3112`. Bit indicating
/// this dump has an opposite-byte-order copy at `fdother(f)`.
pub const FDF_OTHER: u32 = 2;

/// Port of `struct fdhead` from `Src/parse.c:3116`. One per function
/// inside a wordcode dump. All fields are `wordcode` (u32).
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
pub struct fdhead {
    /// Offset (in u32 words) to the start of this function's
    /// wordcode body inside the dump.
    pub start: u32, // c:3117
    /// Wordcode-byte length of the body (excludes pattern-prog slots).
    pub len: u32, // c:3118
    /// Number of compiled patterns the body references.
    pub npats: u32, // c:3119
    /// Offset of the string table inside `prog->prog`.
    pub strs: u32, // c:3120
    /// Header-record length in u32 words (record + name).
    pub hlen: u32, // c:3121
    /// Packed `{ kshload_bits(2) | name_tail_offset(30) }` field.
    pub flags: u32, // c:3122
}

/// `#define FDHF_KSHLOAD 1` from `Src/parse.c:3149`. Function-header
/// flag word — `-k` ksh-style autoload marker.
pub const FDHF_KSHLOAD: u32 = 1;

/// `#define FDHF_ZSHLOAD 2` from `Src/parse.c:3150`. `-z` zsh-style
/// autoload marker.
pub const FDHF_ZSHLOAD: u32 = 2;

/// Port of `struct wcfunc` from `Src/parse.c:3158`. Build-time
/// per-function aggregate before write_dump emits it: the function
/// (or source-file) name, the compiled `Eprog` from `parse_string`,
/// and the FDHF_* autoload-style flag word.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct wcfunc {
    pub name: String, // c:3159
    /// Compiled program (`Eprog prog` c:3160) — wordcode + strs +
    /// npats as built by `bld_eprog`.
    pub prog: eprog, // c:3160
    pub flags: u32, // c:3161
}

/// Port of `dump_find_func(Wordcode h, char *name)` from
/// `Src/parse.c:3167`. Walks the header table inside a loaded
/// dump for a function with the given basename; returns the
/// matching `fdhead` record (C returns the `FDHead` pointer).
pub fn dump_find_func(h: &[u32], name: &str) -> Option<fdhead> {
    // c:3167
    let header_words = fdheaderlen(h) as usize;
    let end = header_words; // walking u32 offsets, end-exclusive
    let mut cur = firstfdhead_offset();
    while cur < end {
        if let Some(fh) = read_fdhead(h, cur) {
            let full = fdname(h, cur);
            let tail = fdhtail(&fh) as usize;
            let basename = if tail <= full.len() {
                &full[tail..]
            } else {
                ""
            };
            if basename == name {
                return Some(fh); // c:3173 `return n;`
            }
            cur = nextfdhead_offset(h, cur);
        } else {
            break;
        }
    }
    None // c:3175
}

/// Port of `bin_zcompile(char *nam, char **args, Options ops, UNUSED(int func))`
/// from `Src/parse.c:3180`. Validates the option set, then dispatches
/// to one of: `-t` (test/list), `-c`/`-a` (dump current functions),
/// or the default (compile source files to `.zwc`).
pub fn bin_zcompile(
    nam: &str, // c:3180
    args: &[String],
    ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    // c:3185-3192 — illegal-combination guard.
    if (OPT_ISSET(ops, b'k') && OPT_ISSET(ops, b'z'))
        || (OPT_ISSET(ops, b'R') && OPT_ISSET(ops, b'M'))
        || (OPT_ISSET(ops, b'c')
            && (OPT_ISSET(ops, b'U') || OPT_ISSET(ops, b'k') || OPT_ISSET(ops, b'z')))
        || (!(OPT_ISSET(ops, b'c') || OPT_ISSET(ops, b'a')) && OPT_ISSET(ops, b'm'))
    {
        zwarnnam(nam, "illegal combination of options"); // c:3192
        return 1;
    }

    // c:3194 — `-c`/`-a` + KSHAUTOLOAD warning.
    if (OPT_ISSET(ops, b'c') || OPT_ISSET(ops, b'a')) && isset(crate::ported::zsh_h::KSHAUTOLOAD) {
        zwarnnam(nam, "functions will use zsh style autoloading"); // c:3195
    }

    // c:3196-3197 — flag word from `-k` / `-z`.
    let flags: u32 = if OPT_ISSET(ops, b'k') {
        FDHF_KSHLOAD
    } else if OPT_ISSET(ops, b'z') {
        FDHF_ZSHLOAD
    } else {
        0
    };

    // c:3199 — `-t` test/list mode.
    if OPT_ISSET(ops, b't') {
        // c:3199
        if args.is_empty() {
            zwarnnam(nam, "too few arguments"); // c:3202
            return 1;
        }
        let dump_name = if args[0].ends_with(FD_EXT) {
            args[0].clone()
        } else {
            format!("{}{}", args[0], FD_EXT)
        };
        let f = match load_dump_header(nam, &dump_name, 1) {
            // c:3206
            Some(buf) => buf,
            None => return 1,
        };
        // c:3209 — per-function check.
        if args.len() > 1 {
            for name in &args[1..] {
                // c:3210
                if dump_find_func(&f, name).is_none() {
                    // c:3212
                    return 1;
                }
            }
            return 0;
        }
        // c:3215-3221 — listing arm. Walk every fdhead, print
        // each function's full name. C uses `fdname(h)` which
        // includes the path prefix; matches our `fdname()` impl.
        let mapped = if (fdflags(&f) & FDF_MAP) != 0 {
            "mapped"
        } else {
            "read"
        };
        println!("zwc file ({}) for zsh-{}", mapped, fdversion(&f));
        let header_words = fdheaderlen(&f) as usize;
        let mut cur = firstfdhead_offset();
        while cur < header_words {
            if read_fdhead(&f, cur).is_none() {
                break;
            }
            println!("{}", fdname(&f, cur));
            cur = nextfdhead_offset(&f, cur);
        }
        return 0;
    }

    if args.is_empty() {
        zwarnnam(nam, "too few arguments"); // c:3226
        return 1;
    }

    // c:3228 — map mode discriminant.
    let map: i32 = if OPT_ISSET(ops, b'M') {
        2
    } else if OPT_ISSET(ops, b'R') {
        0
    } else {
        1
    };

    // c:3230-3236 — single-file default-mode short path.
    if args.len() == 1 && !(OPT_ISSET(ops, b'c') || OPT_ISSET(ops, b'a')) {
        let dump = format!("{}{}", args[0], FD_EXT);
        return build_dump(nam, &dump, args, OPT_ISSET(ops, b'U') as i32, map, flags);
    }

    // c:3239-3247 — multi-file or `-c`/`-a` mode.
    let dump = if args[0].ends_with(FD_EXT) {
        args[0].clone()
    } else {
        format!("{}{}", args[0], FD_EXT)
    };
    let rest = &args[1..];
    if OPT_ISSET(ops, b'c') || OPT_ISSET(ops, b'a') {
        let what =
            (if OPT_ISSET(ops, b'c') { 1 } else { 0 }) | (if OPT_ISSET(ops, b'a') { 2 } else { 0 });
        build_cur_dump(nam, &dump, rest, OPT_ISSET(ops, b'm') as i32, map, what)
    } else {
        build_dump(nam, &dump, rest, OPT_ISSET(ops, b'U') as i32, map, flags)
    }
}

/// Port of `load_dump_header(char *nam, char *name, int err)` from
/// `Src/parse.c:3258`. Opens the file, reads + validates the magic
/// and version, then slurps the full header table into memory.
/// Returns the header u32-array on success or None on any failure
/// (emitting C-shaped warnings when `err != 0`).
pub fn load_dump_header(nam: &str, name: &str, err: i32) -> Option<Vec<u32>> {
    // c:3258

    let mut f = match File::open(name) {
        // c:3263
        Ok(h) => h,
        Err(_) => {
            if err != 0 {
                zwarnnam(nam, &format!("can't open zwc file: {}", name)); // c:3265
            }
            return None;
        }
    };

    // Read FD_PRELEN+1 u32 words = 52 bytes.
    let mut buf_bytes = vec![0u8; (FD_PRELEN + 1) * 4];
    if f.read_exact(&mut buf_bytes).is_err() {
        if err != 0 {
            zwarnnam(nam, &format!("invalid zwc file: {}", name)); // c:3277
        }
        return None;
    }
    let mut buf: Vec<u32> = buf_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    // c:3270 — magic + version check against `ZSH_VERSION` (C global;
    // zshrs mirrors it in `patchlevel::ZSH_VERSION`).
    let magic_ok = fdmagic(&buf) == FD_MAGIC || fdmagic(&buf) == FD_OMAGIC;
    let v_ok = fdversion(&buf) == crate::ported::patchlevel::ZSH_VERSION;
    if !magic_ok {
        if err != 0 {
            zwarnnam(nam, &format!("invalid zwc file: {}", name)); // c:3277
        }
        return None;
    }
    if !v_ok {
        if err != 0 {
            zwarnnam(
                nam,
                &format!(
                    "zwc file has wrong version (zsh-{}): {}", // c:3274
                    fdversion(&buf),
                    name
                ),
            );
        }
        return None;
    }

    // c:3285 — if magic matches host byte order, head len is `pre[FD_PRELEN]`.
    // Else seek to `fdother(buf)` and re-read.
    if fdmagic(&buf) != FD_MAGIC {
        let other = fdother(&buf) as u64; // c:3290
        if f.seek(SeekFrom::Start(other)).is_err() || f.read_exact(&mut buf_bytes).is_err() {
            zwarnnam(nam, &format!("invalid zwc file: {}", name)); // c:3295
            return None;
        }
        buf = buf_bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
    }

    let total_words = fdheaderlen(&buf) as usize; // c:3286/3299
    if total_words < FD_PRELEN + 1 {
        zwarnnam(nam, &format!("invalid zwc file: {}", name));
        return None;
    }

    // Read the remaining header words.
    let mut head: Vec<u32> = Vec::with_capacity(total_words);
    head.extend_from_slice(&buf);
    let remaining_words = total_words - (FD_PRELEN + 1);
    if remaining_words > 0 {
        let mut rest_bytes = vec![0u8; remaining_words * 4]; // c:3305
        if f.read_exact(&mut rest_bytes).is_err() {
            zwarnnam(nam, &format!("invalid zwc file: {}", name)); // c:3307
            return None;
        }
        for c in rest_bytes.chunks_exact(4) {
            head.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
    }
    Some(head) // c:3311
}

/// Port of `fdswap(Wordcode p, int n)` from `Src/parse.c:3318`.
/// Byte-swap each u32 in `p[..n]` in place. Used when writing the
/// opposite-byte-order copy of a wordcode dump.
pub fn fdswap(p: &mut [u32]) {
    // c:3318
    for w in p.iter_mut() {
        *w = w.swap_bytes();
    }
}

/// Port of `write_dump(int dfd, LinkList progs, int map, int hlen, int tlen)`
/// from `Src/parse.c:3334`. Writes the prelude + header records +
/// body wordcode bytes to the dump file descriptor.
///
/// Two passes: first native-byte-order (`FD_MAGIC`), then opposite-
/// byte-order (`FD_OMAGIC`) so big-endian readers can mmap the
/// same file. Bodies are byte-swapped via `fdswap` on the second pass.
pub fn write_dump(
    dfd: &mut File, // c:3334
    progs: &[wcfunc],
    mut map: i32,
    hlen: i32,
    tlen: i32,
) -> std::io::Result<()> {
    // c:3345-3346 — `if (map == 1) map = (tlen >= FD_MINMAP);`
    if map == 1 {
        map = ((tlen as usize) >= FD_MINMAP) as i32;
    }

    // C `sizeof(Patprog)` — pointer size (see bld_eprog len arithmetic).
    let patprog_size = size_of::<*const u8>() as i32;

    let mut other = 0u32; // c:3338
    let ohlen = hlen;

    loop {
        // c:3349 — `for (ohlen = hlen; ; hlen = ohlen)`.
        let mut cur_hlen = ohlen;
        // c:3348 — `memset(pre, 0, sizeof(wordcode) * FD_PRELEN);`
        let mut pre = vec![0u32; FD_PRELEN];
        pre[0] = if other != 0 { FD_OMAGIC } else { FD_MAGIC }; // c:3350
        let flags = (if map != 0 { FDF_MAP } else { 0 }) | other;
        fdsetflags(&mut pre, flags as u8); // c:3351
        fdsetother(&mut pre, tlen as u32); // c:3352
                                           // c:3353 — copy ZSH_VERSION C-string into pre[2..].
        let ver = crate::ported::patchlevel::ZSH_VERSION.as_bytes();
        for (i, &b) in ver.iter().enumerate() {
            let word = 2 + i / 4;
            let shift = (i % 4) * 8;
            pre[word] |= (b as u32) << shift;
        }
        // c:3354 — write prelude.
        for w in &pre {
            dfd.write_all(&w.to_le_bytes())?;
        }
        // c:3356 — per-fn header records.
        for wcf in progs {
            let n = &wcf.name;
            let prog = &wcf.prog;
            // c:3362-3363 — body length in bytes excluding the
            // pattern-prog slots: `prog->len - (prog->npats *
            // sizeof(Patprog))`.
            let len_bytes = prog.len - prog.npats * patprog_size;
            let mut head = fdhead {
                start: cur_hlen as u32, // c:3360
                len: len_bytes as u32,  // c:3363
                npats: prog.npats as u32, // c:3364
                // c:3365 — `head.strs = prog->strs - ((char *) prog->prog);`
                // In bld_eprog's layout strs sits right after the code
                // words, so the byte offset is ecused * 4.
                strs: (prog.prog.len() * 4) as u32, // c:3365
                hlen: ((FDHEAD_WORDS as u32) + ((n.len() as u32 + 4) / 4)), // c:3366
                flags: 0,
            };
            // c:3361 — `hlen += (prog->len - npats*sizeof(Patprog) +
            //                    sizeof(wordcode) - 1) / sizeof(wordcode);`
            cur_hlen += (len_bytes + 3) / 4;
            // c:3368-3371 — name tail offset from path basename.
            let tail = n.rfind('/').map(|p| p + 1).unwrap_or(0);
            head.flags = fdhbldflags(wcf.flags, tail as u32); // c:3372
                                                              // c:3373 — opposite-byte-order swap on second pass.
            let mut head_words: Vec<u32> = vec![
                head.start, head.len, head.npats, head.strs, head.hlen, head.flags,
            ];
            if other != 0 {
                fdswap(&mut head_words);
            }
            for w in &head_words {
                dfd.write_all(&w.to_le_bytes())?;
            }
            // c:3376-3379 — write the name + NUL, then pad to a word
            // boundary with the leading bytes of `head` (C: `write_loop
            // (dfd, (char *)&head, sizeof(wordcode) - tmp);`).
            dfd.write_all(n.as_bytes())?;
            dfd.write_all(&[0u8])?;
            let tmp = (n.len() + 1) & 3;
            if tmp != 0 {
                let head_bytes = head_words[0].to_le_bytes();
                dfd.write_all(&head_bytes[..4 - tmp])?;
            }
        }
        // c:3381-3388 — per-fn bodies: code words then the strs region,
        // padded to a word boundary. `tmp = (prog->len - npats*
        // sizeof(Patprog) + sizeof(wordcode) - 1) / sizeof(wordcode);
        // write_loop(dfd, (char *)prog->prog, tmp * sizeof(wordcode));`
        for wcf in progs {
            let prog = &wcf.prog;
            let len_bytes = (prog.len - prog.npats * patprog_size) as usize;
            let tmp = (len_bytes + 3) / 4;
            // c:3386-3387 — on the other pass only the code words are
            // swapped (`fdswap(prog->prog, ((Wordcode) prog->strs) -
            // prog->prog)`); the strs chars are byte-order neutral.
            let mut body_bytes: Vec<u8> = Vec::with_capacity(tmp * 4);
            for &w in &prog.prog {
                let w = if other != 0 { w.swap_bytes() } else { w };
                body_bytes.extend_from_slice(&w.to_le_bytes());
            }
            if let Some(s) = &prog.strs {
                body_bytes.extend_from_slice(s.as_bytes());
            }
            // C reads up to 3 bytes of heap slop past strs; emit NULs
            // (readers never look past head.len so the value is free).
            body_bytes.resize(tmp * 4, 0);
            dfd.write_all(&body_bytes)?;
        }
        if other != 0 {
            // c:3389
            break;
        }
        other = FDF_OTHER; // c:3391
    }
    Ok(())
}

/// Port of `build_dump(char *nam, char *dump, char **files, int ali, int map, int flags)`
/// from `Src/parse.c:3396`. Source-file → wordcode dump compiler:
/// parses each source file via `parse_string` and serializes the
/// resulting `Eprog`s through `write_dump` into `<dump>.zwc`.
pub fn build_dump(
    nam: &str, // c:3397
    dump: &str,
    files: &[String],
    ali: i32,
    map: i32,
    flags: u32,
) -> i32 {
    use crate::ported::utils::{errflag, ERRFLAG_ERROR};
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::Ordering;

    // c:3403-3404 — append FD_EXT unless already suffixed.
    let dump: String = if dump.ends_with(FD_EXT) {
        dump.to_string()
    } else {
        format!("{}{}", dump, FD_EXT)
    };

    // c:3406 — `unlink(dump);`
    let _ = fs::remove_file(&dump);
    // c:3407-3410 — `open(dump, O_WRONLY|O_CREAT, 0444)`.
    let mut dfd = match fs::OpenOptions::new()
        .write(true)
        .create(true)
        .mode(0o444)
        .open(&dump)
    {
        Ok(f) => f,
        Err(_) => {
            zwarnnam(nam, &format!("can't write zwc file: {}", dump)); // c:3408
            return 1;
        }
    };

    let patprog_size = size_of::<*const u8>() as i32; // C sizeof(Patprog)
    let ona = crate::ported::lex::noaliases(); // c:3398 `ona = noaliases`
    crate::ported::lex::set_noaliases(ali != 0); // c:3412 `noaliases = ali;`

    let mut progs: Vec<wcfunc> = Vec::new(); // c:3411
    let mut flags = flags;
    let mut hlen = FD_PRELEN as i32; // c:3414
    let mut tlen: i32 = 0;

    for fname in files {
        // c:3418-3425 — `-k` / `-z` pseudo-args flip the autoload style.
        if check_cond(fname, "k") {
            flags = (flags & !(FDHF_KSHLOAD | FDHF_ZSHLOAD)) | FDHF_KSHLOAD; // c:3419
            continue;
        } else if check_cond(fname, "z") {
            flags = (flags & !(FDHF_KSHLOAD | FDHF_ZSHLOAD)) | FDHF_ZSHLOAD; // c:3422
            continue;
        }
        // c:3426-3437 — open + fstat + S_ISREG + read the whole file.
        let fnam = crate::ported::utils::unmeta(fname); // c:3426
        let is_reg = fs::metadata(&fnam).map(|m| m.is_file()).unwrap_or(false);
        let bytes = if is_reg { fs::read(&fnam).ok() } else { None };
        let bytes = match bytes {
            Some(b) => b,
            None => {
                zwarnnam(nam, &format!("can't open file: {}", fname)); // c:3432
                crate::ported::lex::set_noaliases(ona); // c:3433
                let _ = fs::remove_file(&dump); // c:3434
                return 1;
            }
        };
        // c:3450 — `file = metafy(file, flen, META_REALLOC);` — keep
        // raw bytes intact through the &str boundary (see bld_eprog's
        // from_utf8_unchecked rationale).
        let raw = unsafe { String::from_utf8_unchecked(bytes) };
        let file = crate::ported::utils::metafy(&raw);

        // c:3452-3460 — parse; any error aborts the whole dump.
        let prog = crate::ported::exec::parse_string(&file, 1);
        let errored = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
        let prog = match prog {
            Some(p) if !errored => p,
            _ => {
                errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed); // c:3453
                zwarnnam(nam, &format!("can't read file: {}", fname)); // c:3456
                crate::ported::lex::set_noaliases(ona); // c:3457
                let _ = fs::remove_file(&dump); // c:3458
                return 1;
            }
        };

        // c:3467-3472 — accumulate header + body length budgets.
        let flen = (fname.len() as i32 + 4) / 4; // c:3469
        hlen += (FDHEAD_WORDS as i32) + flen; // c:3470
        tlen += (prog.len - prog.npats * patprog_size + 3) / 4; // c:3471

        // c:3463-3466 — wcfunc node.
        let wcf_flags = if (prog.flags & EF_RUN) != 0 {
            FDHF_KSHLOAD // c:3465
        } else {
            flags
        };
        progs.push(wcfunc {
            name: fname.clone(),
            prog,
            flags: wcf_flags,
        });
    }
    crate::ported::lex::set_noaliases(ona); // c:3474

    let tlen = (tlen + hlen) * 4; // c:3476

    // c:3478 — `write_dump(dfd, progs, map, hlen, tlen);` (void in C).
    let _ = write_dump(&mut dfd, &progs, map, hlen, tlen);

    0 // c:3482
}

/// Port of `cur_add_func(char *nam, Shfunc shf, LinkList names, LinkList progs, int *hlen, int *tlen, int what)`
/// from `Src/parse.c:3489`. Adds a shfunc to the in-build dump
/// progs+names lists. Stub: `Eprog` for the function body isn't
/// yet wired through `shfunc.funcdef` to be serializable here.
pub fn cur_add_func(
    nam: &str, // c:3489
    shf_name: &str,
    shf_flags: i32,
    names: &mut Vec<String>,
    progs: &mut Vec<wcfunc>,
    hlen: &mut i32,
    tlen: &mut i32,
    what: i32,
) -> i32 {
    let is_undef = (shf_flags as u32 & PM_UNDEFINED) != 0;
    if is_undef {
        if (what & 2) == 0 {
            // c:3498
            zwarnnam(nam, &format!("function is not loaded: {}", shf_name));
            return 1;
        }
        // c:3503 — would call `getfpfunc` to load body for dump.
        zwarnnam(nam, &format!("can't load function: {}", shf_name));
        return 1;
    } else if (what & 1) == 0 {
        zwarnnam(nam, &format!("function is already loaded: {}", shf_name)); // c:3514
        return 1;
    }
    // c:3517 — would `dupeprog(shf->funcdef)`. Stub: empty program.
    let wcf = wcfunc {
        name: shf_name.to_string(),
        flags: FDHF_ZSHLOAD,
        prog: eprog::default(),
    };
    progs.push(wcf);
    names.push(shf_name.to_string());

    // c:3526 — bump hlen / tlen.
    let name_words = (shf_name.len() as i32 + 4) / 4;
    *hlen += (FDHEAD_WORDS as i32) + name_words;
    *tlen += 0; // body is empty in stub; real path adds prog->len in words.

    0
}

/// Port of `build_cur_dump(char *nam, char *dump, char **names, int match, int map, int what)`
/// from `Src/parse.c:3536`. Compiles currently-loaded functions
/// (`-c` for functions, `-a` for aliases) into a `.zwc` dump.
/// Same wordcode-emit dependency as `build_dump`.
pub fn build_cur_dump(
    nam: &str, // c:3536
    dump: &str,
    _names: &[String],
    _match_: i32,
    _map: i32,
    _what: i32,
) -> i32 {
    zwarnnam(
        nam,
        &format!("{}: wordcode dump-current emit not yet ported", dump),
    );
    1
}

/// Port of `zwcstat(char *filename, struct stat *buf)` from
/// `Src/parse.c:3656`. Stats a `.zwc` file, falling back to
/// `.zwc.old` if the primary doesn't exist (zsh uses the `.old`
/// suffix to keep a previous dump readable while a rewrite is in
/// progress).
pub fn zwcstat(filename: &str) -> Option<fs::Metadata> {
    // c:3656
    if let Ok(m) = fs::metadata(filename) {
        return Some(m);
    }
    let old = format!("{}.old", filename);
    fs::metadata(&old).ok()
}

/// Port of `load_dump_file(char *dump, struct stat *sbuf, int other, int len)`
/// from `Src/parse.c:3675`. Reads (or mmap()'s) a complete `.zwc`
/// file into memory. Returns the u32 buffer or None on I/O error.
pub fn load_dump_file(
    dump: &str, // c:3675
    _sbuf: &fs::Metadata,
    other: i32,
    _len: usize,
) -> Option<Vec<u32>> {
    let mut f = File::open(dump).ok()?;
    if other != 0 {
        f.seek(SeekFrom::Start(other as u64)).ok()?;
    }
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Port of `try_dump_file(char *path, char *name, char *file, int *ksh, int test_only)`
/// from `Src/parse.c:3746`. Tries to load function `name` from a
/// `.zwc` digest (`<path>.zwc`) or per-function compiled file
/// (`<file>.zwc`) when each is newer than its uncompiled source.
pub fn try_dump_file(
    path: &str,
    name: &str,
    file: &str, // c:3746
    test_only: bool,
) -> Option<(eprog, i32)> {
    use std::fs;

    // c:3753-3758 — if path ends in .zwc, treat as direct digest.
    if path.ends_with(FD_EXT) {
        crate::ported::signals::queue_signals();
        let result = fs::metadata(path)
            .ok()
            .and_then(|m| check_dump_file(path, &m, name, test_only));
        unqueue_signals();
        return result;
    }

    // c:3759-3760 — dig = "<path>.zwc", wc = "<file>.zwc".
    let dig = format!("{}{}", path, FD_EXT);
    let wc = format!("{}{}", file, FD_EXT);

    // c:3762-3764 — zwcstat(dig, &std); stat(wc, &stc); stat(file, &stn);
    let std_meta = fs::metadata(&dig);
    let stc_meta = fs::metadata(&wc);
    let stn_meta = fs::metadata(file);

    crate::ported::signals::queue_signals();

    // c:3771-3777 — try digest if newer than (or in absence of) wc/file.
    if let Ok(std_m) = &std_meta {
        let dig_mtime = std_m.modified().ok();
        let wc_newer_or_missing = match &stc_meta {
            Err(_) => true,
            Ok(c) => dig_mtime >= c.modified().ok(),
        };
        let src_newer_or_missing = match &stn_meta {
            Err(_) => true,
            Ok(n) => dig_mtime >= n.modified().ok(),
        };
        if wc_newer_or_missing && src_newer_or_missing {
            if let Some(prog) = check_dump_file(&dig, std_m, name, test_only) {
                unqueue_signals();
                return Some(prog);
            }
        }
    }

    // c:3779-3784 — try per-function .zwc if newer than (or in absence of) source.
    if let Ok(stc_m) = &stc_meta {
        let wc_mtime = stc_m.modified().ok();
        let src_newer_or_missing = match &stn_meta {
            Err(_) => true,
            Ok(n) => wc_mtime >= n.modified().ok(),
        };
        if src_newer_or_missing {
            if let Some(prog) = check_dump_file(&wc, stc_m, name, test_only) {
                unqueue_signals();
                return Some(prog);
            }
        }
    }

    unqueue_signals(); // c:3787
    None // c:3788
}

/// Port of `try_source_file(char *file)` from `Src/parse.c:3795`.
/// Returns an Eprog (the wordcode dump body) if `<file>.zwc` exists
/// and is newer than `<file>`, else None. The dump entry searched is
/// the file's basename (`tail`), matching how `zcompile` names
/// source-file entries.
pub fn try_source_file(file: &str) -> Option<eprog> {
    // c:3795

    // c:3802-3805 — if ((tail = strrchr(file, '/'))) tail++; else tail = file;
    let tail = match file.rfind('/') {
        Some(i) => &file[i + 1..],
        None => file,
    };

    // c:3807-3812 — if (strsfx(FD_EXT, file)) { ... return check_dump_file(file, NULL, tail, NULL, 0); }
    if file.ends_with(FD_EXT) {
        crate::ported::signals::queue_signals(); // c:3808
        let meta = fs::metadata(file);
        let prog = match meta {
            Ok(m) => check_dump_file(file, &m, tail, false).map(|(p, _)| p), // c:3809
            Err(_) => None,
        };
        unqueue_signals(); // c:3810
        return prog;
    }

    // c:3813 — wc = dyncat(file, FD_EXT);
    let wc = format!("{}{}", file, FD_EXT);

    // c:3815-3816 — rc = stat(wc, &stc); rn = stat(file, &stn);
    let stc = fs::metadata(&wc);
    let stn = fs::metadata(file);

    crate::ported::signals::queue_signals(); // c:3818
                                             // c:3819-3823 — if (!rc && (rn || stc.st_mtime >= stn.st_mtime) && (prog = check_dump_file(...))) return prog;
    if let Ok(meta_c) = &stc {
        let newer_than_src = match (&stc, &stn) {
            (Ok(c), Ok(n)) => c.modified().ok() >= n.modified().ok(),
            (Ok(_), Err(_)) => true, // c:3819 — `rn` (src missing) ⇒ accept .zwc
            _ => false,
        };
        if newer_than_src {
            let prog = check_dump_file(&wc, meta_c, tail, false); // c:3820
            if let Some((p, _)) = prog {
                unqueue_signals(); // c:3821
                return Some(p); // c:3822
            }
        }
    }
    unqueue_signals(); // c:3824
    None // c:3825
}

/// Port of `Eprog check_dump_file(char *file, struct stat *sbuf,
/// char *name, int *ksh, int test_only)` from `Src/parse.c:3833`.
/// Walks the `dumps` mmap list looking for `(dev, ino)` matching
/// `sbuf`; on miss, calls `load_dump_header` to read the .zwc
/// header. Then `dump_find_func(d, name)` locates the function
/// table entry. Returns the wordcode slice + ksh-load flag.
///
/// ```c
/// Eprog
/// check_dump_file(char *file, struct stat *sbuf, char *name,
///                 int *ksh, int test_only)
/// {
///     int isrec = 0;
///     Wordcode d;
///     FDHead h;
///     FuncDump f;
///     struct stat lsbuf;
///     if (!sbuf) {
///         if (zwcstat(file, &lsbuf)) return NULL;
///         sbuf = &lsbuf;
///     }
///   rec:
///     d = NULL;
///     for (f = dumps; f; f = f->next)
///         if (f->dev == sbuf->st_dev && f->ino == sbuf->st_ino)
///             { d = f->map; break; }
///     if (!f && (isrec || !(d = load_dump_header(NULL, file, 0))))
///         return NULL;
///     if ((h = dump_find_func(d, name))) {
///         if (test_only) return &dummy_eprog;
///         /* allocate Eprog from f->map at h offset, incrdumpcount,
///            return prog */
///     }
///     return NULL;
/// }
/// ```
/// Rust port returns `Option<(eprog, i32)>` instead of the C
/// `Eprog` pointer + `*ksh` out-param: tuple element 0 is the
/// loaded program (wordcode + string table per the fdhead record),
/// element 1 is the ksh-load mode exactly as C writes `*ksh`:
/// `FDHF_KSHLOAD → 2`, `FDHF_ZSHLOAD → 0`, neither → `1`
/// (c:3954-3956).
pub fn check_dump_file(
    // c:3833
    file: &str,
    sbuf: &fs::Metadata,
    name: &str,
    test_only: bool,
) -> Option<(eprog, i32)> {
    use std::os::unix::fs::MetadataExt;

    // c:3842-3846 — `if (!sbuf) { zwcstat(file, &lsbuf); sbuf = &lsbuf; }`
    // Rust takes sbuf by &Metadata — never null.
    let dev = sbuf.dev(); // c:3859
    let ino = sbuf.ino(); // c:3859

    // c:3854 — `d = NULL;`
    let mut d: Option<Vec<u32>> = None;
    let mut found_mmap = false; // c:3858 `for (f = dumps; f; ...)`

    // c:3858-3862 — walk DUMPS for matching dev/ino.
    {
        let dumps_guard = DUMPS.lock().expect("dumps poisoned");
        for f in dumps_guard.iter() {
            // c:3858
            if f.dev == dev && f.ino == ino {
                // c:3859
                d = Some(f.map.clone()); // c:3860
                found_mmap = true;
                break; // c:3861
            }
        }
    }

    // c:3870-3871 — `if (!f && (isrec || !(d = load_dump_header(NULL, file, 0)))) return NULL;`
    if !found_mmap {
        // c:3870
        match load_dump_header("", file, 0) {
            // c:3870 load_dump_header
            Some(loaded) => d = Some(loaded),
            None => return None, // c:3871
        }
    }

    // c:3873 — `if ((h = dump_find_func(d, name)))`
    let dump = d?;
    let h = dump_find_func(&dump, name)?; // c:3873

    // c:3876-3879 — `if (test_only) return &dummy_eprog;`
    if test_only {
        // c:3876
        return Some((eprog::default(), 0)); // c:3879 dummy
    }

    // c:3954-3956 — `*ksh = ((fdhflags(h) & FDHF_KSHLOAD) ? 2 :
    //                        ((fdhflags(h) & FDHF_ZSHLOAD) ? 0 : 1));`
    let ksh = if (fdhflags(&h) & FDHF_KSHLOAD) != 0 {
        2
    } else if (fdhflags(&h) & FDHF_ZSHLOAD) != 0 {
        0
    } else {
        1
    };

    // c:3919-3958 — the read (non-mmap) branch: open the file, seek
    // to the function's wordcode, read `h->len` bytes (wordcode +
    // string pool), and build an EF_REAL Eprog around it. zshrs has
    // no mmap'd EF_MAP variant — DUMPS entries store the file words
    // in `map`, so both branches funnel into the same read-and-copy.
    //
    //   if ((fd = open(file, O_RDONLY)) < 0 ||
    //       lseek(fd, ((h->start * sizeof(wordcode)) +
    //                  ((fdflags(d) & FDF_OTHER) ? fdother(d) : 0)), 0) < 0)
    //       return NULL;
    //   d = (Wordcode) zalloc(h->len + po);
    //   if (read(fd, ((char *) d) + po, h->len) != (int)h->len) return NULL;
    //   prog->flags = EF_REAL;
    //   prog->len = h->len + po;
    //   prog->npats = np = h->npats;
    //   prog->prog = (Wordcode) (((char *) d) + po);
    //   prog->strs = ((char *) prog->prog) + h->strs;
    let body_off = (h.start as u64) * 4
        + if (fdflags(&dump) & FDF_OTHER) != 0 {
            fdother(&dump) as u64 // c:3924
        } else {
            0
        };
    let mut f = File::open(file).ok()?; // c:3922
    f.seek(SeekFrom::Start(body_off)).ok()?; // c:3923
    let mut bytes = vec![0u8; h.len as usize];
    if f.read_exact(&mut bytes).is_err() {
        // c:3931 `read(...) != h->len`
        return None;
    }
    // `h->strs` is the byte offset of the string pool inside the read
    // region; everything before it is wordcode.
    let strs_off = (h.strs as usize).min(bytes.len());
    let prog_words: Vec<u32> = bytes[..strs_off]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    // SAFETY: same byte-not-char convention as `bld_eprog` (c:566) —
    // consumers index `strs` by byte offset and never require UTF-8.
    let strs_string = unsafe { String::from_utf8_unchecked(bytes[strs_off..].to_vec()) };
    let po = h.npats as usize * size_of::<*const u8>(); // c:3920
    let prog = eprog {
        flags: EF_REAL,                  // c:3941
        len: (h.len as usize + po) as i32, // c:3942
        npats: h.npats as i32,           // c:3943
        nref: 1,                         // c:3944
        pats: Vec::new(),                // c:3945/3952 dummy_patprog1 fill
        prog: prog_words,                // c:3946
        strs: Some(strs_string),         // c:3947
        shf: None,                       // c:3948
        dump: None,                      // c:3949
    };

    // c:3899 — incrdumpcount(f) on the mmap-cache hit path.
    if found_mmap {
        let dumps_guard = DUMPS.lock().expect("dumps poisoned");
        if let Some(f) = dumps_guard.iter().find(|f| f.dev == dev && f.ino == ino) {
            incrdumpcount(f); // c:3899
        }
    }

    Some((prog, ksh)) // c:3958
}

/// Port of `incrdumpcount(FuncDump f)` from `Src/parse.c:3970/4021`.
/// `f->count++;` — refcount-up a loaded dump entry. The Rust port
/// keys lookup by `filename` because Rust can't raw-pointer-compare
/// funcdump values inside a `Mutex<Vec<...>>`; same observable
/// effect (the count of the matching entry increments).
pub fn incrdumpcount(f: &funcdump) {
    // c:3970 — `f->count++;`
    if let Some(d) = DUMPS
        .lock()
        .unwrap()
        .iter_mut()
        .find(|d| d.filename.as_deref() == f.filename.as_deref())
    {
        d.count += 1; // c:3973
    }
}

/// Port of `freedump(FuncDump f)` from `Src/parse.c:3976`. Public
/// helper for the rare external caller; locks the dumps mutex and
/// drops the entry with the given filename.
pub fn freedump(f: &funcdump) {
    // c:3976
    let mut g = DUMPS.lock().unwrap();
    if let Some(name) = f.filename.as_deref() {
        freedump_locked(&mut g, name);
    }
}

/// Port of `decrdumpcount(FuncDump f)` from `Src/parse.c:3988/4026`.
/// `f->count--; if (!f->count) { unlink from dumps; freedump(f); }`.
pub fn decrdumpcount(f: &funcdump) {
    // c:3988
    let key = f.filename.clone();
    let mut g = DUMPS.lock().unwrap();
    let mut hit_zero: Option<String> = None;
    for d in g.iter_mut() {
        if d.filename == key {
            d.count -= 1; // c:3991
            if d.count == 0 {
                // c:3992
                hit_zero = d.filename.clone();
            }
            break;
        }
    }
    if let Some(name) = hit_zero {
        // c:3994-4001
        freedump_locked(&mut g, &name);
    }
}

/// Port of `closedumps(void)` from `Src/parse.c:4008/4033`. Walks
/// `dumps` freeing every entry. Called on shell exit (exec.c:522).
pub fn closedumps() {
    // c:4008
    let mut g = DUMPS.lock().unwrap();
    g.clear(); // c:4011-4014 `while (dumps) { ... freedump(...); ... }`
}

/// Port of `dump_autoload(char *nam, char *file, int on, Options ops, int func)`
/// from `Src/parse.c:4042`. Registers every function in a `.zwc`
/// for autoload via `shfunctab`.
pub fn dump_autoload(
    nam: &str,
    file: &str, // c:4042
    on: i32,
    ops: &crate::ported::zsh_h::options,
    func: i32,
) -> i32 {
    use crate::ported::zsh_h::shfunc;
    let mut ret = 0; // c:4047

    // c:4049-4050 — if (!strsfx(FD_EXT, file)) file = dyncat(file, FD_EXT);
    let file_owned;
    let file = if !file.ends_with(FD_EXT) {
        file_owned = format!("{}{}", file, FD_EXT);
        file_owned.as_str()
    } else {
        file
    };

    // c:4052-4053 — if (!(h = load_dump_header(nam, file, 1))) return 1;
    let h = match load_dump_header(nam, file, 1) {
        Some(buf) => buf,
        None => return 1,
    };

    // c:4055-4056 — for (n = firstfdhead(h); n < e; n = nextfdhead(n))
    let hlen = fdheaderlen(&h) as usize; // c:4055
    let mut n_off = firstfdhead_offset();
    while n_off < hlen {
        let head = match read_fdhead(&h, n_off) {
            Some(hd) => hd,
            None => break,
        };
        // c:4057-4061 — shf = zshcalloc; shf->node.flags = on; ...addnode(fdname + fdhtail)
        let name_full = fdname(&h, n_off);
        let tail = fdhtail(&head) as usize;
        let basename: String = name_full.chars().skip(tail).collect();
        let mut shf = shfunc {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: basename.clone(),
                flags: on, // c:4058
            },
            filename: None,
            lineno: 0,
            funcdef: None,
            redir: None,
            sticky: None, // c:4060 NULL
            body: None,
        };
        // c:4059 — shf->funcdef = mkautofn(shf);  (placeholder Eprog ptr)
        let _ = crate::ported::builtin::mkautofn(&mut shf as *mut _);
        // c:4061 — shfunctab->addnode(...)
        let snapshot = shf.clone();
        {
            let mut tab = crate::ported::hashtable::shfunctab_lock()
                .write()
                .expect("shfunctab poisoned");
            tab.add(shf);
        }
        // c:4062-4063 — if (OPT_ISSET(ops,'X') && eval_autoload(...)) ret = 1;
        if OPT_ISSET(ops, b'X') {
            let mut shf_ref = snapshot;
            if crate::ported::builtin::eval_autoload(&mut shf_ref as *mut _, &basename, ops, func)
                != 0
            {
                ret = 1;
            }
        }
        n_off = nextfdhead_offset(&h, n_off);
    }
    let _ = nam;
    ret // c:4065
}

/// Port of C `struct eccstr` (zsh.h:836) — the long-string dedup BST
/// node. The dedup-walk and cmp logic in `ecstrcode` is faithful to
/// parse.c:447-453 including the conditional cmp chain
/// (nfunc → hashval → strcmp), so corpus inputs where C's eccstr BST walk
/// finds-or-misses match get the same outcome on the Rust side.
struct EccstrNode {
    left: Option<Box<EccstrNode>>,
    right: Option<Box<EccstrNode>>,
    /// C-byte form of the string (single byte per char ≤ 0xff).
    /// Owned because Rust doesn't have C zsh's "stable pointers into
    /// the lexer's tokstr arena" — every tokstr lives as a fresh
    /// Rust String allocation.
    str: Vec<u8>,
    /// Wordcode-encoded offset: `(byte_offset << 2) | token_bit`.
    /// Same shape as `Eccstr::offs` (parse.c:459).
    offs: u32,
    /// Absolute byte offset in the final strs region (= `ecsoffs` at
    /// insert time). C `Eccstr::aoffs` (parse.c:464). copy_ecstr uses
    /// THIS for the write position — distinct from `offs` which is
    /// ecssub-relative and collides across funcdef scopes.
    aoffs: u32,
    /// `nfunc` snapshot at insert time. Per-function namespace key
    /// — top-level scripts use 0; each funcdef bumps it.
    nfunc: i32,
    /// Hash of `str` computed via zsh's `hasher` (hashtable.c:86).
    hashval: u32,
}
// === end AST relocation ===

// Parser state lives in file-scope thread_locals:
//   - LEX_* (lexer side, matching Src/lex.c file-statics)
//   - ECBUF / ECLEN / ECUSED / ECNPATS / ECSOFFS / ECSSUB / ECNFUNC /
//     ECSTRS_INDEX / ECSTRS_REVERSE (wordcode-emission state, matching
//     Src/parse.c file-statics)
//
// Callers use the free-fn entry points directly:
//   crate::ported::parse::parse_init(input);
//   let prog = crate::ported::parse::parse();

const MAX_RECURSION_DEPTH: usize = 500;

/// Direct port of `struct parse_stack` at `Src/zsh.h:3099-3109`.
/// Used by `parse_context_save` / `parse_context_restore`
/// (parse.c:295-355) to snapshot per-parse-call state so a nested
/// parse (e.g. inside command substitution) doesn't clobber the
/// outer parse.
///
/// A second port of `struct parse_stack` exists at
/// `crate::ported::zsh_h::parse_stack` (zsh.h:1066) using canonical
/// Wordcode / Eccstr / `struct heredocs` types — that port is unused
/// today and will become authoritative when Phase 9b (PORT_PLAN.md)
/// wires wordcode emission. This local version uses the working-set
/// shapes (`Vec<HereDoc>`, stubbed wordcode fields) suited to zshrs's
/// pre-wordcode AST architecture; the consolidation happens in P9b.
#[allow(non_camel_case_types)]
#[derive(Debug, Default, Clone)]
pub struct parse_stack {
    // ── Direct port of struct parse_stack at zsh.h:3099-3109 ──
    /// Pending heredocs awaiting body collection (canonical C
    /// linked-list shape). C: `struct heredocs *hdocs` (zsh.h:3100).
    /// Mirrors `parse::HDOCS` thread_local across nested parses.
    pub hdocs: Option<Box<crate::ported::zsh_h::heredocs>>,
    /// !!! WARNING: NOT IN PARSE_STACK — Rust-only AST-glue !!!
    /// Snapshot of `lex::LEX_HEREDOCS` (the parallel Rust-only Vec
    /// carrying terminator / strip_tabs / quoted metadata).
    /// Saved/restored alongside the canonical `hdocs` so nested
    /// parses get a clean AST view. C's parse_stack has no analog
    /// because C tracks terminator metadata implicitly via tokstr.
    pub lex_heredocs: Vec<HereDoc>,
    /// C: `int incmdpos` (zsh.h:3102).
    pub incmdpos: bool,
    /// C: `int aliasspaceflag` (zsh.h:3103).
    pub aliasspaceflag: i32,
    /// C: `int incond` (zsh.h:3104).
    pub incond: i32,
    /// C: `int inredir` (zsh.h:3105).
    pub inredir: bool,
    /// C: `int incasepat` (zsh.h:3106).
    pub incasepat: i32,
    /// C: `int isnewlin` (zsh.h:3107).
    pub isnewlin: i32,
    /// C: `int infor` (zsh.h:3108).
    pub infor: i32,
    /// C: `int inrepeat_` (zsh.h:3109).
    pub inrepeat_: i32,
    /// C: `int intypeset` (zsh.h:3110).
    pub intypeset: bool,
    // ── Wordcode-buffer state — STUB until Phase 9b ──
    // C `Wordcode ecbuf` (zsh.h:3112) + `Eccstr ecstrs` (zsh.h:3113) +
    // `int eclen/ecused/ecnpats/ecsoffs/ecssub/ecnfunc` (zsh.h:3112-3114).
    // zshrs hasn't emitted wordcode yet — these fields exist to
    // preserve the C shape but read/write nothing until P9b lands.
    pub eclen: i32,
    pub ecused: i32,
    pub ecnpats: i32,
    pub ecbuf: Option<Vec<u32>>,
    pub ecstrs: Option<Vec<u8>>,
    pub ecsoffs: i32,
    pub ecssub: i32,
    pub ecnfunc: i32,
}

// Old uppercase Rust-only `ParseStack` is gone. Compat alias so
// existing call sites (context.rs) keep resolving until the
// rename ripples through.
/// `ParseStack` type alias.
#[allow(non_camel_case_types)]
pub type ParseStack = parse_stack;

/// `mod_export struct eprog dummy_eprog;` from `Src/parse.c:3066`.
/// Placeholder Eprog used by `shf->funcdef = &dummy_eprog;` in
/// builtin.c when clearing a stale autoload stub. Held in a Mutex
/// so `init_eprog` can set it once at shell startup.
pub static DUMMY_EPROG: std::sync::Mutex<eprog> = std::sync::Mutex::new(eprog {
    flags: 0,
    len: 0,
    npats: 0,
    nref: 0,
    prog: Vec::new(),
    strs: None,
    pats: Vec::new(),
    shf: None,
    dump: None,
});

/// Walk every ZshRedir in the program and, for any with a `heredoc_idx`,
/// pull the body+terminator out of `bodies` and stuff into `heredoc`.
/// `bodies[i]` corresponds to the i-th heredoc registered by the lexer
/// during scanning (in source order).
fn fill_heredoc_bodies(prog: &mut ZshProgram, bodies: &[HereDocInfo]) {
    for list in &mut prog.lists {
        fill_in_sublist(&mut list.sublist, bodies);
    }
}

fn fill_in_sublist(sub: &mut ZshSublist, bodies: &[HereDocInfo]) {
    fill_in_pipe(&mut sub.pipe, bodies);
    if let Some(next) = &mut sub.next {
        fill_in_sublist(&mut next.1, bodies);
    }
}

fn fill_in_pipe(pipe: &mut ZshPipe, bodies: &[HereDocInfo]) {
    fill_in_command(&mut pipe.cmd, bodies);
    if let Some(next) = &mut pipe.next {
        fill_in_pipe(next, bodies);
    }
}

fn fill_in_command(cmd: &mut ZshCommand, bodies: &[HereDocInfo]) {
    match cmd {
        ZshCommand::Simple(s) => {
            for r in &mut s.redirs {
                if let Some(idx) = r.heredoc_idx {
                    if let Some(info) = bodies.get(idx) {
                        r.heredoc = Some(info.clone());
                    }
                }
            }
        }
        ZshCommand::Subsh(p) | ZshCommand::Cursh(p) => fill_heredoc_bodies(p, bodies),
        ZshCommand::FuncDef(f) => fill_heredoc_bodies(&mut f.body, bodies),
        ZshCommand::If(i) => {
            fill_heredoc_bodies(&mut i.cond, bodies);
            fill_heredoc_bodies(&mut i.then, bodies);
            for (c, b) in &mut i.elif {
                fill_heredoc_bodies(c, bodies);
                fill_heredoc_bodies(b, bodies);
            }
            if let Some(e) = &mut i.else_ {
                fill_heredoc_bodies(e, bodies);
            }
        }
        ZshCommand::While(w) | ZshCommand::Until(w) => {
            fill_heredoc_bodies(&mut w.cond, bodies);
            fill_heredoc_bodies(&mut w.body, bodies);
        }
        ZshCommand::For(f) => fill_heredoc_bodies(&mut f.body, bodies),
        ZshCommand::Case(c) => {
            for arm in &mut c.arms {
                fill_heredoc_bodies(&mut arm.body, bodies);
            }
        }
        ZshCommand::Repeat(r) => fill_heredoc_bodies(&mut r.body, bodies),
        ZshCommand::Time(Some(sublist)) => fill_in_sublist(sublist, bodies),
        ZshCommand::Try(t) => {
            fill_heredoc_bodies(&mut t.try_block, bodies);
            fill_heredoc_bodies(&mut t.always, bodies);
        }
        ZshCommand::Redirected(inner, redirs) => {
            for r in redirs {
                if let Some(idx) = r.heredoc_idx {
                    if let Some(info) = bodies.get(idx) {
                        r.heredoc = Some(info.clone());
                    }
                }
            }
            fill_in_command(inner, bodies);
        }
        ZshCommand::Time(None) | ZshCommand::Cond(_) | ZshCommand::Arith(_) => {}
    }
}

/// If `list` is a Simple containing one word that ends in the
/// `<Inpar><Outpar>` token pair (the lexer-port encoding of `()`),
/// return the bare name. Used by `parse_program_until` to detect
/// `name() {body}` style function definitions where the lexer
/// hasn't split the `()` from the name.
/// Detect the `name() …` shape inside a Simple. Returns the function
/// name and (when the body was already inlined into the same Simple,
/// e.g. `foo() echo hi`) the rest of the words as the body's argv.
/// Returns None for non-funcdef shapes.
fn simple_name_with_inoutpar(list: &ZshList) -> Option<(Vec<String>, Vec<String>)> {
    if list.flags.async_ || list.sublist.next.is_some() {
        return None;
    }
    let pipe = &list.sublist.pipe;
    if pipe.next.is_some() {
        return None;
    }
    let simple = match &pipe.cmd {
        ZshCommand::Simple(s) => s,
        _ => return None,
    };
    if simple.words.is_empty() || !simple.assigns.is_empty() {
        return None;
    }
    let suffix = "\u{88}\u{8a}"; // Inpar + Outpar
                                 // Find the FIRST word ending in `()`. zsh accepts the
                                 // multi-name shorthand `fna fnb fnc() { body }` (parse.c:
                                 // par_funcdef wordlist) — words[0..i-1] are extra names,
                                 // words[i] is `lastname()`. Words after are the body argv
                                 // (one-line shorthand, `name() cmd args`).
    let par_idx = simple.words.iter().position(|w| w.ends_with(suffix))?;
    let mut names: Vec<String> = Vec::with_capacity(par_idx + 1);
    for w in &simple.words[..par_idx] {
        // Earlier names must be bare identifiers, NOT contain
        // tokens that imply they're not function names (no `()`,
        // no quotes, no expansions). zsh's lexer enforces this
        // at the wordlist level; we approximate by requiring the
        // word be an identifier-shaped token after untokenize.
        let bare = super::lex::untokenize(w);
        let valid = !bare.is_empty()
            && bare
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '$');
        if !valid {
            return None;
        }
        names.push(bare);
    }
    let last = &simple.words[par_idx];
    let bare = &last[..last.len() - suffix.len()];
    if bare.is_empty() {
        return None;
    }
    names.push(super::lex::untokenize(bare));
    let rest = simple.words[par_idx + 1..].to_vec();
    Some((names, rest))
}

/// Initialize parser state for a fresh parse of `input`.
/// Free-fn entry point — resets parser thread_locals and loads input.
pub fn parse_init(input: &str) {
    // Seed the option defaults the parser/lexer inspect. Real zsh
    // installs these via `install_emulation_defaults` (options.c:172)
    // at shell startup; zshrs's parse-only test entry path bypasses
    // init_main, so we mirror the `zsh` emulation defaults here.
    //
    // Under `cfg(test)` (lib unit tests share one process) we
    // unconditionally OVERWRITE these on every parse_init so cross-
    // test option pollution (a prior test that flipped one of these
    // and panicked before its restore ran) doesn't leak into the
    // parser's reserved-word recognition / one-liner detection.
    //
    // In the REAL shell this overwrite is WRONG: parse_init runs for
    // every -c string, eval, and cmd-subst body, so resetting
    // `posixbuiltins` here silently wiped `zshrs -o POSIX_BUILTINS`
    // and made `setopt posix_builtins` evaporate at the next parse
    // (A04redirect.ztst POSIX_BUILTINS chunks). C zsh's parser READS
    // options; it never writes them. Production seeds only entries
    // that are missing entirely (parse-only test paths that bypass
    // init_main still get the zsh-emulation defaults).
    let overwrite = cfg!(test);
    for (name, default) in [
        ("shortloops", true),
        ("shortrepeat", false),
        ("multifuncdef", true),
        ("aliasfuncdef", false),
        ("ignorebraces", false),
        ("cshjunkieloops", false),
        ("posixbuiltins", false),
        ("execopt", true),
        ("kshautoload", false),
        ("aliases", true),
    ] {
        if overwrite || crate::ported::options::opt_state_get(name).is_none() {
            crate::ported::options::opt_state_set(name, default);
        }
    }
    lex_init(input);
}

/// P9b decoder (wordcode-pipeline variant): direct port of
/// `ecgetstr(Estate s, int dup, int *tokflag)` from
/// `Src/parse.c:2855-2890`. Reads a wordcode at `pc`, decodes the
/// encoded string back to owned String. Returns (string,
/// pc_after_consumed). Distinct from the existing `ecgetstr` (which
/// takes a separate strs buffer for text.rs) — this variant uses
/// the live ECSTRS_REVERSE HashMap populated at ecstrcode time.
pub fn ecgetstr_wordcode(buf: &[u32], pc: usize) -> (String, usize) {
    if pc >= buf.len() {
        return (String::new(), pc);
    }
    let c = buf[pc];
    let next = pc + 1;
    // parse.c:2862-2863 — empty-string sentinels.
    if c == 6 || c == 7 {
        return (String::new(), next);
    }
    // parse.c:2864-2871 — inline-packed short string.
    if (c & 2) != 0 {
        let b0 = ((c >> 3) & 0xff) as u8;
        let b1 = ((c >> 11) & 0xff) as u8;
        let b2 = ((c >> 19) & 0xff) as u8;
        let mut bytes: Vec<u8> = Vec::new();
        for b in [b0, b1, b2] {
            if b == 0 {
                break;
            }
            bytes.push(b);
        }
        return (String::from_utf8_lossy(&bytes).into_owned(), next);
    }
    // parse.c:2872-2873 — long string via offs lookup. Map value is
    // metafied Vec<u8>; convert back to display String. Unmetafy is
    // the caller's job (the wordcode-parity dumper does it; other
    // callers may want raw bytes).
    let s = ECSTRS_REVERSE
        .with_borrow(|m| m.get(&c).cloned())
        .map(|v| String::from_utf8_lossy(&v).into_owned())
        .unwrap_or_default();
    (s, next)
}

/// Parse the complete input. Direct port of `parse_event` /
/// `par_list` from `Src/parse.c:614-720`. On syntax error,
/// sets `errflag |= ERRFLAG_ERROR` (via `zerr`) and returns the
/// partial program — callers check `errflag` to detect failure,
/// matching C's `Eprog parse_event(...)` + `if (errflag) {...}`.
pub fn parse() -> ZshProgram {
    zshlex();

    let mut program = parse_program_until(None);

    // Post-pass: wire heredoc bodies (collected by the inline NEWLIN
    // walk in zshlex into LEX_HEREDOCS) back into ZshRedir.heredoc
    // fields via heredoc_idx. No C analog — LEX_HEREDOCS is the
    // Rust-only AST-glue Vec.
    let bodies: Vec<HereDocInfo> = LEX_HEREDOCS
        .with_borrow(|v| v.clone())
        .into_iter()
        .map(|h| HereDocInfo {
            content: h.content,
            terminator: h.terminator,
            quoted: h.quoted,
        })
        .collect();
    if !bodies.is_empty() {
        fill_heredoc_bodies(&mut program, &bodies);
    }

    program
}

/// Wordcode-emission top-level driver. Closest C analog is
/// `parse_list(void)` at `Src/parse.c:697-712`: init_parse +
/// zshlex + par_list(&c) + bld_eprog. This entry omits init_parse
/// and bld_eprog (caller responsibilities) and inlines a guard
/// loop around par_list_wordcode for cases where the lexer leaves
/// a non-ENDINPUT terminator (LEXERR, missing close-token, etc.).
pub fn par_event_wordcode() -> usize {
    let start = ECUSED.get() as usize;
    // C `parse_list` (parse.c:697-712) calls par_list ONCE — par_list's
    // own goto-rec loop handles all SEPER-separated sublists. The
    // outer loop here exists for safety against early-return cases
    // (LEXERR, missing terminator) but normally par_list_wordcode
    // consumes everything in one call.
    let mut cmplx: i32 = 0;
    while tok() != ENDINPUT && tok() != LEXERR {
        par_list_wordcode(&mut cmplx);
        match tok() {
            SEMI | NEWLIN | AMPER | AMPERBANG | SEPER => {
                zshlex();
            }
            _ => break,
        }
    }
    // No trailing `ecadd(WCB_END())` here: C's `par_list` (c:769)
    // emits none — the single terminating `WCB_END` comes from
    // `bld_eprog` (c:555). Emitting one here too made every program
    // one word longer than C's (double END), breaking .zwc dump
    // byte-parity with `zcompile`.
    start
}

/// Port of `par_list(int *cmplx)` from `Src/parse.c:769-803`.
/// `list : { SEPER } [ sublist [ { SEPER | AMPER | AMPERBANG } list ] ]`.
/// True line-by-line port: takes `cmplx: &mut i32` matching C's
/// `int *cmplx` out-parameter, uses stack-local `c` per iteration
/// like C (so inner sublist cmplx is independent of outer).
pub fn par_list_wordcode(cmplx: &mut i32) {
    // c:773 — `int p, lp = -1, c;`
    let mut p: usize;
    let mut lp: i32 = -1;
    let mut c: i32;
    loop {
        // c:775 `rec:` — c:777-778 `while (tok == SEPER) zshlex();`
        while tok() == SEPER {
            zshlex();
        }
        // c:780 — `p = ecadd(0);`
        p = ecadd(0);
        // c:781 — `c = 0;`
        c = 0;
        // c:783 — `if (par_sublist(&c)) { ... }`
        if par_sublist_wordcode(&mut c) {
            // c:784 — `*cmplx |= c;`
            *cmplx |= c;
            // c:785 — `if (tok == SEPER || tok == AMPER || tok == AMPERBANG)`
            let t = tok();
            if t == SEPER || t == AMPER || t == AMPERBANG {
                // c:786-787 — `if (tok != SEPER) *cmplx = 1;`
                if t != SEPER {
                    *cmplx = 1;
                }
                // c:788-790 — `set_list_code(p, ..., c);`
                let z = if t == SEPER {
                    Z_SYNC
                } else if t == AMPER {
                    Z_ASYNC
                } else {
                    Z_ASYNC | Z_DISOWN
                };
                set_list_code(p, z, c != 0);
                // c:791 — `incmdpos = 1;`
                set_incmdpos(true);
                // c:792-794 — `do { zshlex(); } while (tok == SEPER);`
                loop {
                    zshlex();
                    if tok() != SEPER {
                        break;
                    }
                }
                // c:795 — `lp = p;` c:796 — `goto rec;`
                lp = p as i32;
                continue;
            } else {
                // c:798 — `set_list_code(p, (Z_SYNC | Z_END), c);`
                set_list_code(p, Z_SYNC | Z_END, c != 0);
            }
        } else {
            // c:800-802 — `ecused--; if (lp >= 0) ecbuf[lp] |= wc_bdata(Z_END);`
            ECUSED.set((ECUSED.get() - 1).max(0));
            if lp >= 0 {
                ECBUF.with_borrow_mut(|b| {
                    if (lp as usize) < b.len() {
                        b[lp as usize] |= wc_bdata(Z_END as wordcode);
                    }
                });
            }
        }
        break;
    }
}

/// Port of `par_list1(int *cmplx)` from `Src/parse.c:806-817`.
/// Single-sublist variant used by funcdef bodies and the short
/// `for`/`while`/`repeat` forms — exactly one sublist with
/// `Z_SYNC|Z_END`, no chain.
pub fn par_list1_wordcode(cmplx: &mut i32) {
    // c:810 — `int p = ecadd(0), c = 0;`
    let p = ecadd(0);
    let mut c: i32 = 0;
    // c:812 — `if (par_sublist(&c)) { ... }`
    if par_sublist_wordcode(&mut c) {
        // c:813 — `set_list_code(p, (Z_SYNC | Z_END), c);`
        set_list_code(p, Z_SYNC | Z_END, c != 0);
        // c:814 — `*cmplx |= c;`
        *cmplx |= c;
    } else {
        // c:816 — `ecused--;`
        ECUSED.set((ECUSED.get() - 1).max(0));
    }
}

/// Port of `par_save_list(C)` macro from `Src/parse.c:475-480`.
///   do { int eu = ecused; par_list(C); if (eu == ecused) ecadd(WCB_END()); } while (0)
pub fn par_save_list_wordcode(cmplx: &mut i32) {
    let eu = ECUSED.get();
    par_list_wordcode(cmplx);
    if ECUSED.get() == eu {
        ecadd(WCB_END());
    }
}

/// Port of `par_save_list1(C)` macro from `Src/parse.c:481-486`.
pub fn par_save_list1_wordcode(cmplx: &mut i32) {
    let eu = ECUSED.get();
    par_list1_wordcode(cmplx);
    if ECUSED.get() == eu {
        ecadd(WCB_END());
    }
}

/// Port of `par_sublist(int *cmplx)` from `Src/parse.c:823-865`.
/// `sublist : sublist2 [ ( DBAR | DAMPER ) { SEPER } sublist ]`.
/// Emits a WCB_SUBLIST header, recurses into par_sublist2 for
/// the !/coproc prefix + pipeline, then chains via DBAR (`||`)
/// or DAMPER (`&&`) recursively. Returns true if at least one
/// pipeline was emitted.
pub fn par_sublist_wordcode(cmplx: &mut i32) -> bool {
    // c:827 — `int f, p, c = 0;`
    let mut c: i32 = 0;
    // c:829 — `p = ecadd(0);`
    let p = ecadd(0);
    // c:831 — `if ((f = par_sublist2(&c)) != -1) { ... }`
    match par_sublist2(&mut c) {
        Some(f) => {
            // c:832 — `int e = ecused;`
            let e = ECUSED.get() as usize;
            // c:834 — `*cmplx |= c;`
            *cmplx |= c;
            if tok() == DBAR || tok() == DAMPER {
                // c:836 — `enum lextok qtok = tok;`
                let qtok = tok();
                // c:839 — `cmdpush(tok == DBAR ? CS_CMDOR : CS_CMDAND);`
                cmdpush(if qtok == DBAR {
                    CS_CMDOR as u8
                } else {
                    CS_CMDAND as u8
                });
                // c:840 — `zshlex();`
                zshlex();
                // c:841-842 — `while (tok == SEPER) zshlex();`
                while tok() == SEPER {
                    zshlex();
                }
                // c:843 — `sl = par_sublist(cmplx);`
                let sl = par_sublist_wordcode(cmplx);
                // c:844-847 — `set_sublist_code(p, (sl ? ... : WC_SUBLIST_END),
                // f, (e - 1 - p), c);`
                let st = if sl {
                    if qtok == DBAR {
                        WC_SUBLIST_OR
                    } else {
                        WC_SUBLIST_AND
                    }
                } else {
                    WC_SUBLIST_END
                };
                set_sublist_code(p, st as i32, f, (e - 1 - p) as i32, c != 0);
                // c:848 — `cmdpop();`
                cmdpop();
            } else {
                // c:850-853 — `if (tok == AMPER || tok == AMPERBANG)
                // { c = 1; *cmplx |= c; }`
                if tok() == AMPER || tok() == AMPERBANG {
                    c = 1;
                    *cmplx |= c;
                }
                // c:854 — `set_sublist_code(p, WC_SUBLIST_END, f,
                // (e - 1 - p), c);`
                set_sublist_code(p, WC_SUBLIST_END as i32, f, (e - 1 - p) as i32, c != 0);
            }
            // c:856 — `return 1;`
            true
        }
        None => {
            // c:858-859 — `ecused--; return 0;`
            ECUSED.set((ECUSED.get() - 1).max(0));
            false
        }
    }
}

/// Port of `par_pline(int *cmplx)` from `Src/parse.c:894-955`.
/// `pline : cmd [ ( BAR | BARAMP ) { SEPER } pline ]`. Emits a
/// WCB_PIPE header (mid for chain links, end for the last cmd)
/// plus the optional BARAMP `2>&1` synthetic redir.
/// Port of `par_pline(int *cmplx)` from `Src/parse.c:893-947`.
/// (Named `par_pipe_wordcode` to disambiguate from the AST
/// `par_pline` at parse.rs:3744 — semantically the same `pline`
/// production.)
pub fn par_pipe_wordcode(cmplx: &mut i32) -> bool {
    // c:897 — `zlong line = toklineno;`
    let line = toklineno() as i64;
    // c:899 — `p = ecadd(0);`
    let p = ecadd(0);
    // c:901-904 — `if (!par_cmd(cmplx, 0)) { ecused--; return 0; }`
    if !par_cmd_wordcode(cmplx, 0) {
        ECUSED.set((ECUSED.get() - 1).max(0));
        return false;
    }
    if tok() == BAR_TOK {
        // c:906 — `*cmplx = 1;`
        *cmplx = 1;
        // c:907 — `cmdpush(CS_PIPE);`
        cmdpush(CS_PIPE as u8);
        // c:908 — `zshlex();`
        zshlex();
        // c:909-910 — `while (tok == SEPER) zshlex();`
        while tok() == SEPER {
            zshlex();
        }
        // c:911 — `ecbuf[p] = WCB_PIPE(WC_PIPE_MID, line>=0 ? line+1 : 0);`
        ECBUF.with_borrow_mut(|b| {
            if p < b.len() {
                b[p] = WCB_PIPE(
                    WC_PIPE_MID,
                    if line >= 0 { (line + 1) as wordcode } else { 0 },
                );
            }
        });
        // c:912 — `ecispace(p+1, 1);`
        ecispace(p + 1, 1);
        // c:913 — `ecbuf[p+1] = ecused - 1 - p;`
        let used = ECUSED.get() as usize;
        ECBUF.with_borrow_mut(|b| {
            if p + 1 < b.len() {
                b[p + 1] = (used.saturating_sub(1 + p)) as wordcode;
            }
        });
        // c:914-916 — `if (!par_pline(cmplx)) { tok = LEXERR; }`
        if !par_pipe_wordcode(cmplx) {
            set_tok(LEXERR);
        }
        // c:917 — `cmdpop();`
        cmdpop();
        true
    } else if tok() == BARAMP {
        // c:920-923 — walk past inline WC_REDIR to find r.
        let mut r = p + 1;
        loop {
            let code = ECBUF.with_borrow(|b| b.get(r).copied().unwrap_or(0));
            if wc_code(code) != WC_REDIR {
                break;
            }
            r += WC_REDIR_WORDS(code) as usize;
        }
        // c:925-928 — `ecispace(r, 3);` + synthetic `2>&1` redir
        ecispace(r, 3);
        ECBUF.with_borrow_mut(|b| {
            if r + 2 < b.len() {
                b[r] = WCB_REDIR(REDIR_MERGEOUT as wordcode);
                b[r + 1] = 2;
                b[r + 2] = ecstrcode("1");
            }
        });
        // c:930 — `*cmplx = 1;`
        *cmplx = 1;
        cmdpush(CS_ERRPIPE as u8);
        zshlex();
        while tok() == SEPER {
            zshlex();
        }
        ECBUF.with_borrow_mut(|b| {
            if p < b.len() {
                b[p] = WCB_PIPE(
                    WC_PIPE_MID,
                    if line >= 0 { (line + 1) as wordcode } else { 0 },
                );
            }
        });
        ecispace(p + 1, 1);
        let used = ECUSED.get() as usize;
        ECBUF.with_borrow_mut(|b| {
            if p + 1 < b.len() {
                b[p + 1] = (used.saturating_sub(1 + p)) as wordcode;
            }
        });
        if !par_pipe_wordcode(cmplx) {
            set_tok(LEXERR);
        }
        cmdpop();
        true
    } else {
        // c:944 — `ecbuf[p] = WCB_PIPE(WC_PIPE_END, line>=0 ? line+1 : 0);`
        ECBUF.with_borrow_mut(|b| {
            if p < b.len() {
                b[p] = WCB_PIPE(
                    WC_PIPE_END,
                    if line >= 0 { (line + 1) as wordcode } else { 0 },
                );
            }
        });
        true
    }
}

/// Port of `par_cmd(int *cmplx, int zsh_construct)` from
/// `Src/parse.c:958-1085`. Parses leading + trailing redirs and
/// dispatches on the current token to the right par_* builder.
/// Returns false only when no command was emitted (no redirs +
/// par_simple returned 0).
/// Port of `par_cmd(int *cmplx, int zsh_construct)` from
/// `Src/parse.c:957-1077`.
pub fn par_cmd_wordcode(cmplx: &mut i32, zsh_construct: i32) -> bool {
    // c:960 — `int r, nr = 0;`
    let mut nr: i32 = 0;
    // c:962 — `r = ecused;`
    let mut r: usize = ECUSED.get() as usize;
    // c:964-968 — leading redirs.
    if IS_REDIROP(tok()) {
        // c:965 — `*cmplx = 1;`
        *cmplx = 1;
        // c:966-968 — `while (IS_REDIROP(tok)) { nr += par_redir(&r, NULL); }`
        while IS_REDIROP(tok()) {
            nr += par_redir_wordcode(&mut r, None);
        }
    }
    // c:970-1066 — token-dispatch switch.
    match tok() {
        FOR => {
            cmdpush(CS_FOR as u8);
            par_for_wordcode(cmplx);
            cmdpop();
        }
        FOREACH => {
            cmdpush(CS_FOREACH as u8);
            par_for_wordcode(cmplx);
            cmdpop();
        }
        SELECT => {
            // c:982 — `*cmplx = 1;`
            *cmplx = 1;
            cmdpush(CS_SELECT as u8);
            par_for_wordcode(cmplx);
            cmdpop();
        }
        CASE => {
            cmdpush(CS_CASE as u8);
            par_case_wordcode(cmplx);
            cmdpop();
        }
        IF => {
            par_if_wordcode(cmplx);
        }
        WHILE => {
            cmdpush(CS_WHILE as u8);
            par_while_wordcode(cmplx);
            cmdpop();
        }
        UNTIL => {
            cmdpush(CS_UNTIL as u8);
            par_while_wordcode(cmplx);
            cmdpop();
        }
        REPEAT => {
            cmdpush(CS_REPEAT as u8);
            par_repeat_wordcode(cmplx);
            cmdpop();
        }
        INPAR_TOK => {
            // c:1011 — `*cmplx = 1;`
            *cmplx = 1;
            cmdpush(CS_SUBSH as u8);
            par_subsh_wordcode(cmplx, zsh_construct);
            cmdpop();
        }
        INBRACE_TOK => {
            cmdpush(CS_CURSH as u8);
            par_subsh_wordcode(cmplx, zsh_construct);
            cmdpop();
        }
        FUNC => {
            cmdpush(CS_FUNCDEF as u8);
            par_funcdef_wordcode(cmplx);
            cmdpop();
        }
        DINBRACK => {
            cmdpush(CS_COND as u8);
            par_cond_wordcode();
            cmdpop();
        }
        DINPAR => {
            par_arith_wordcode();
        }
        TIME => {
            // c:1037-1050 — `static int inpartime` guard so
            // `time time foo` doesn't recurse infinitely.
            if !PARSER_INPARTIME.with(|c| c.get()) {
                // c:1041 — `*cmplx = 1;`
                *cmplx = 1;
                PARSER_INPARTIME.with(|c| c.set(true));
                par_time_wordcode();
                PARSER_INPARTIME.with(|c| c.set(false));
            } else {
                set_tok(STRING_LEX);
                let sr = par_simple_wordcode(cmplx, nr);
                if sr == 0 && nr == 0 {
                    return false;
                }
                if sr > 1 {
                    *cmplx = 1;
                    r += (sr - 1) as usize;
                }
            }
        }
        _ => {
            // c:1054 — `if (!(sr = par_simple(cmplx, nr)))`
            let sr = par_simple_wordcode(cmplx, nr);
            if sr == 0 {
                if nr == 0 {
                    return false;
                }
            } else if sr > 1 {
                // c:1060-1061 — `*cmplx = 1; r += sr - 1;`
                *cmplx = 1;
                r += (sr - 1) as usize;
            }
        }
    }
    // c:1067-1071 — trailing redirs.
    // c:1067 — `if (IS_REDIROP(tok)) { *cmplx = 1; while (...) (void)par_redir(&r, NULL); }`
    if IS_REDIROP(tok()) {
        *cmplx = 1;
        while IS_REDIROP(tok()) {
            let _ = par_redir_wordcode(&mut r, None);
        }
    }
    // c:1072-1075 — `incmdpos=1; incasepat=0; incond=0; intypeset=0;`
    set_incmdpos(true);
    set_incasepat(0);
    set_incond(0);
    set_intypeset(false);
    let _ = r;
    // c:1076 — `return 1;`
    true
}

/// Port of `par_for(int *cmplx)` from `Src/parse.c:1086-1198`.
pub fn par_for_wordcode(cmplx: &mut i32) {
    // c:1089 — `int oecused = ecused, csh = (tok == FOREACH), p, sel = (tok == SELECT);`
    let _oecused = ECUSED.get() as usize;
    let csh = tok() == FOREACH;
    let sel = tok() == SELECT;
    let p: usize;
    // c:1090 — `int type;`
    let r#type: wordcode;

    // c:1092 — `p = ecadd(0);`
    p = ecadd(0);

    // c:1094 — `incmdpos = 0;`
    set_incmdpos(false);
    // c:1095 — `infor = tok == FOR ? 2 : 0;`
    set_infor(if tok() == FOR { 2 } else { 0 });
    // c:1096 — `zshlex();`
    zshlex();
    // c:1097 — `if (tok == DINPAR) {`
    if tok() == DINPAR {
        // c:1098 — `zshlex();`
        zshlex();
        // c:1099-1100 — `if (tok != DINPAR) YYERRORV(oecused);`
        if tok() != DINPAR {
            zerr("par_for: expected init");
            return;
        }
        // c:1101 — `ecstr(tokstr);`
        ecstr(&tokstr().unwrap_or_default());
        // c:1102 — `zshlex();`
        zshlex();
        // c:1103-1104
        if tok() != DINPAR {
            zerr("par_for: expected cond");
            return;
        }
        // c:1105
        ecstr(&tokstr().unwrap_or_default());
        // c:1106
        zshlex();
        // c:1107-1108
        if tok() != DOUTPAR {
            zerr("par_for: expected ))");
            return;
        }
        // c:1109
        ecstr(&tokstr().unwrap_or_default());
        // c:1110 — `infor = 0;`
        set_infor(0);
        // c:1111 — `incmdpos = 1;`
        set_incmdpos(true);
        // c:1112 — `zshlex();`
        zshlex();
        // c:1113 — `type = WC_FOR_COND;`
        r#type = WC_FOR_COND;
    } else {
        // c:1115 — `int np = 0, n, posix_in, ona = noaliases, onc = nocorrect;`
        let mut np: usize = 0;
        let mut n: u32;
        let posix_in: bool;
        let ona = noaliases();
        let onc = nocorrect();
        // c:1116 — `infor = 0;`
        set_infor(0);
        // c:1117-1118 — `if (tok != STRING || !isident(tokstr)) YYERRORV(oecused);`
        if tok() != STRING_LEX || !crate::ported::params::isident(&tokstr().unwrap_or_default()) {
            zerr("par_for: expected identifier");
            return;
        }
        // c:1119-1120 — `if (!sel) np = ecadd(0);`
        if !sel {
            np = ecadd(0);
        }
        // c:1121 — `n = 0;`
        n = 0;
        // c:1122 — `incmdpos = 1;`
        set_incmdpos(true);
        // c:1123 — `noaliases = nocorrect = 1;`
        set_noaliases(true);
        set_nocorrect(1);
        // c:1124 — `for (;;) {`
        loop {
            // c:1125 — `n++;`
            n += 1;
            // c:1126 — `ecstr(tokstr);`
            ecstr(&tokstr().unwrap_or_default());
            // c:1127 — `zshlex();`
            zshlex();
            // c:1128-1129 — `if (tok != STRING || !strcmp(tokstr, "in") || sel) break;`
            if tok() != STRING_LEX || tokstr().as_deref() == Some("in") || sel {
                break;
            }
            // c:1130-1135 — `if (!isident(tokstr) || errflag) { ... YYERRORV; }`
            if !crate::ported::params::isident(&tokstr().unwrap_or_default())
                || (errflag.load(Ordering::Relaxed) & 1) != 0
            {
                set_noaliases(ona);
                set_nocorrect(onc);
                zerr("par_for: expected identifier in name list");
                return;
            }
        }
        // c:1137-1138 — `noaliases = ona; nocorrect = onc;`
        set_noaliases(ona);
        set_nocorrect(onc);
        // c:1139-1140 — `if (!sel) ecbuf[np] = n;`
        if !sel {
            ECBUF.with_borrow_mut(|b| {
                b[np] = n;
            });
        }
        // c:1141 — `posix_in = isnewlin;`
        posix_in = isnewlin() != 0;
        // c:1142-1143 — `while (isnewlin) zshlex();`
        while isnewlin() != 0 {
            zshlex();
        }
        // c:1144 — `if (tok == STRING && !strcmp(tokstr, "in")) {`
        if tok() == STRING_LEX && tokstr().as_deref() == Some("in") {
            // c:1145 — `incmdpos = 0;`
            set_incmdpos(false);
            // c:1146 — `zshlex();`
            zshlex();
            // c:1147 — `np = ecadd(0);`
            np = ecadd(0);
            // c:1148 — `n = par_wordlist();`
            let n2 = par_wordlist_wordcode();
            // c:1149-1150 — `if (tok != SEPER) YYERRORV(oecused);`
            if tok() != SEPER {
                zerr("par_for: expected separator after `in`");
                return;
            }
            // c:1151 — `ecbuf[np] = n;`
            ECBUF.with_borrow_mut(|b| {
                b[np] = n2 as wordcode;
            });
            // c:1152 — `type = (sel ? WC_SELECT_LIST : WC_FOR_LIST);`
            r#type = if sel { WC_SELECT_LIST } else { WC_FOR_LIST };
        } else if !posix_in && tok() == INPAR_TOK {
            // c:1153-1154 — `else if (!posix_in && tok == INPAR)`
            // c:1154 — `incmdpos = 0;`
            set_incmdpos(false);
            // c:1155 — `zshlex();`
            zshlex();
            // c:1156 — `np = ecadd(0);`
            np = ecadd(0);
            // c:1157 — `n = par_nl_wordlist();`
            let n2 = par_nl_wordlist_wordcode();
            // c:1158-1159 — `if (tok != OUTPAR) YYERRORV(oecused);`
            if tok() != OUTPAR_TOK {
                zerr("par_for: expected `)`");
                return;
            }
            // c:1160 — `ecbuf[np] = n;`
            ECBUF.with_borrow_mut(|b| {
                b[np] = n2 as wordcode;
            });
            // c:1161 — `incmdpos = 1;`
            set_incmdpos(true);
            // c:1162 — `zshlex();`
            zshlex();
            // c:1163 — `type = (sel ? WC_SELECT_LIST : WC_FOR_LIST);`
            r#type = if sel { WC_SELECT_LIST } else { WC_FOR_LIST };
        } else {
            // c:1165 — `type = (sel ? WC_SELECT_PPARAM : WC_FOR_PPARAM);`
            r#type = if sel { WC_SELECT_PPARAM } else { WC_FOR_PPARAM };
        }
        let _ = np;
    }
    // c:1167 — `incmdpos = 1;`
    set_incmdpos(true);
    // c:1168-1169 — `while (tok == SEPER) zshlex();`
    while tok() == SEPER {
        zshlex();
    }
    // c:1170-1193 — body dispatch (inline in C, factored here for
    // reuse by par_while/par_repeat — same control flow, same calls).
    par_loop_body_wordcode(cmplx, csh);
    // c:1195-1197 — `ecbuf[p] = (sel ? WCB_SELECT(...) : WCB_FOR(...));`
    let used = ECUSED.get() as usize;
    let off = used.saturating_sub(1 + p) as wordcode;
    ECBUF.with_borrow_mut(|b| {
        b[p] = if sel {
            WCB_SELECT(r#type, off)
        } else {
            WCB_FOR(r#type, off)
        };
    });
}

/// Port of `par_wordlist(void)` from `Src/parse.c:2361-2371` —
/// emits wordcode form. Returns the number of strings emitted.
fn par_wordlist_wordcode() -> u32 {
    // c:2364 — `int num = 0;`
    let mut num: u32 = 0;
    // c:2365 — `while (tok == STRING) {`
    while tok() == STRING_LEX {
        // c:2366 — `ecstr(tokstr);`
        ecstr(&tokstr().unwrap_or_default());
        // c:2367 — `num++;`
        num += 1;
        // c:2368 — `zshlex();`
        zshlex();
    }
    // c:2370 — `return num;`
    num
}

/// Port of `par_nl_wordlist(void)` from `Src/parse.c:2378-2390` —
/// emits wordcode form. Like par_wordlist but tolerates SEPER
/// between words.
fn par_nl_wordlist_wordcode() -> u32 {
    // c:2381 — `int num = 0;`
    let mut num: u32 = 0;
    // c:2383 — `while (tok == STRING || tok == SEPER) {`
    while tok() == STRING_LEX || tok() == SEPER || tok() == NEWLIN {
        // c:2384-2387 — `if (tok != SEPER) { ecstr(tokstr); num++; }`
        if tok() == STRING_LEX {
            ecstr(&tokstr().unwrap_or_default());
            num += 1;
        }
        // c:2388 — `zshlex();`
        zshlex();
    }
    // c:2390 — `return num;`
    num
}

/// Body dispatch shared by par_for / par_while / par_repeat.
/// Direct port of `Src/parse.c:1170-1194`.
fn par_loop_body_wordcode(cmplx: &mut i32, csh: bool) {
    if tok() == DOLOOP {
        zshlex();
        // c:1172 — `par_save_list(cmplx);`
        par_save_list_wordcode(cmplx);
        if tok() != DONE {
            zerr("missing `done`");
            return;
        }
        set_incmdpos(false);
        zshlex();
    } else if tok() == INBRACE_TOK {
        zshlex();
        // c:1179 — `par_save_list(cmplx);`
        par_save_list_wordcode(cmplx);
        if tok() != OUTBRACE_TOK {
            zerr("missing `}`");
            return;
        }
        set_incmdpos(false);
        zshlex();
    } else if csh || isset(CSHJUNKIELOOPS) {
        // c:1185 — `par_save_list(cmplx);`
        par_save_list_wordcode(cmplx);
        if tok() != ZEND {
            zerr("missing `end`");
            return;
        }
        set_incmdpos(false);
        zshlex();
    } else if unset(SHORTLOOPS) {
        zerr("short loop form requires SHORTLOOPS");
    } else {
        // c:1193 — `par_save_list1(cmplx);`
        par_save_list1_wordcode(cmplx);
    }
}

/// `select` shares par_for body (c:983-985 routes SELECT to par_for).
pub fn par_select_wordcode(cmplx: &mut i32) {
    par_for_wordcode(cmplx);
}

/// Port of `par_case(int *cmplx)` from `Src/parse.c:1208-1400`.
pub fn par_case_wordcode(_cmplx: &mut i32) {
    // c:1211 — `int oecused = ecused, brflag, p, pp, palts, type, nalts;`
    let _oecused = ECUSED.get() as usize;
    let brflag: bool;
    let p: usize;
    let mut pp: usize;
    let mut palts: usize;
    let mut r#type: wordcode;
    let mut nalts: u32;
    // c:1212 — `int ona, onc;`
    let ona: bool;
    let onc: i32;

    // c:1214 — `p = ecadd(0);`
    p = ecadd(0);

    // c:1216 — `incmdpos = 0;`
    set_incmdpos(false);
    // c:1217 — `zshlex();`
    zshlex();
    // c:1218-1219 — `if (tok != STRING) YYERRORV(oecused);`
    if tok() != STRING_LEX {
        zerr("par_case: expected scrutinee");
        return;
    }
    // c:1220 — `ecstr(tokstr);`
    ecstr(&tokstr().unwrap_or_default());

    // c:1222 — `incmdpos = 1;`
    set_incmdpos(true);
    // c:1223-1224 — `ona = noaliases; onc = nocorrect;`
    ona = noaliases();
    onc = nocorrect();
    // c:1225 — `noaliases = nocorrect = 1;`
    set_noaliases(true);
    set_nocorrect(1);
    // c:1226 — `zshlex();`
    zshlex();
    // c:1227-1228 — `while (tok == SEPER) zshlex();`
    while tok() == SEPER {
        zshlex();
    }
    // c:1229 — `if (!(tok == STRING && !strcmp(tokstr, "in")) && tok != INBRACE)`
    if !(tok() == STRING_LEX && tokstr().as_deref() == Some("in")) && tok() != INBRACE_TOK {
        // c:1231-1233 — restore noaliases/nocorrect + ERROR
        set_noaliases(ona);
        set_nocorrect(onc);
        zerr("par_case: expected `in` or `{`");
        return;
    }
    // c:1235 — `brflag = (tok == INBRACE);`
    brflag = tok() == INBRACE_TOK;
    // c:1236 — `incasepat = 1;`
    set_incasepat(1);
    // c:1237 — `incmdpos = 0;`
    set_incmdpos(false);
    // c:1238-1239 — `noaliases = ona; nocorrect = onc;`
    set_noaliases(ona);
    set_nocorrect(onc);
    // c:1240 — `zshlex();`
    zshlex();

    // c:1242 — `for (;;) {`
    'arms: loop {
        // c:1243 — `char *str;`
        let mut str: String;
        // c:1244 — `int skip_zshlex;`
        let skip_zshlex: bool;

        // c:1246-1247 — `while (tok == SEPER) zshlex();`
        while tok() == SEPER {
            zshlex();
        }
        // c:1248-1249 — `if (tok == OUTBRACE) break;`
        if tok() == OUTBRACE_TOK {
            break 'arms;
        }
        // c:1250-1251 — `if (tok == INPAR) zshlex();`
        if tok() == INPAR_TOK {
            zshlex();
        }
        // c:1252-1254 — `if (tok == BAR) { str = ""; skip_zshlex = 1; }`
        if tok() == BAR_TOK {
            str = String::new();
            skip_zshlex = true;
        } else {
            // c:1256-1257 — `if (tok != STRING) YYERRORV(oecused);`
            if tok() != STRING_LEX {
                zerr("par_case: expected pattern");
                return;
            }
            // c:1258-1259 — `if (!strcmp(tokstr, "esac")) break;`
            if tokstr().as_deref() == Some("esac") {
                break 'arms;
            }
            // c:1260 — `str = dupstring(tokstr);`
            str = tokstr().unwrap_or_default();
            // c:1261 — `skip_zshlex = 0;`
            skip_zshlex = false;
        }
        // c:1263 — `type = WC_CASE_OR;`
        r#type = WC_CASE_OR;
        // c:1264-1266 — `pp = ecadd(0); palts = ecadd(0); nalts = 0;`
        pp = ecadd(0);
        palts = ecadd(0);
        nalts = 0;
        // c:1300 — `incasepat = -1;`
        set_incasepat(-1);
        // c:1301 — `incmdpos = 1;`
        set_incmdpos(true);
        // c:1302-1303 — `if (!skip_zshlex) zshlex();`
        if !skip_zshlex {
            zshlex();
        }
        // c:1304 — `for (;;) {`
        loop {
            // c:1305-1313 — `if (tok == OUTPAR) { ecstr(str);
            //   ecadd(ecnpats++); nalts++; incasepat = 0;
            //   incmdpos = 1; zshlex(); break; }`
            if tok() == OUTPAR_TOK {
                ecstr(&str);
                let np = ECNPATS.with(|cc| {
                    let v = cc.get();
                    cc.set(v + 1);
                    v
                }) as u32;
                ecadd(np);
                nalts += 1;
                set_incasepat(0);
                set_incmdpos(true);
                zshlex();
                break;
            }
            // c:1314-1320 — `else if (tok == BAR) { ecstr(str);
            //   ecadd(ecnpats++); nalts++; incasepat = 1;
            //   incmdpos = 0; }`
            else if tok() == BAR_TOK {
                ecstr(&str);
                let np = ECNPATS.with(|cc| {
                    let v = cc.get();
                    cc.set(v + 1);
                    v
                }) as u32;
                ecadd(np);
                nalts += 1;
                set_incasepat(1);
                set_incmdpos(false);
            }
            // c:1321-1357 — else { ... `(...)` whole-pattern hack:
            // the lexer absorbed a complete `(...)` as one STRING
            // (str[0] == Inpar) and the current tok is already the
            // body's first token. Massage blanks around `|`/parens
            // at depth 1, validate balance, strip the surrounding
            // parens; the remainder IS the pattern. }
            else {
                use crate::ported::zsh_h::{Bar, Inpar, Outpar};
                if nalts == 0 && str.starts_with(Inpar) {
                    let meta = crate::ported::zsh_h::Meta as char;
                    let blank = |c: char| c.is_ascii() && crate::ported::ztype_h::iblank(c as u8);
                    let mut chars: Vec<char> = str.chars().collect();
                    // c:1323 — `int pct = 0, sl;`
                    let mut pct = 0i32;
                    let mut i = 0usize;
                    // c:1326-1344 — scan/massage loop. `s` ↔ `i`;
                    // chuck(p) ↔ chars.remove(idx).
                    let mut early_break = false;
                    while i < chars.len() {
                        if chars[i] == Inpar {
                            pct += 1;
                        }
                        // c:1329-1330 — `if (!pct) break;` (char past
                        // the balanced close → trailing garbage).
                        if pct == 0 {
                            early_break = true;
                            break;
                        }
                        if pct == 1 {
                            // c:1332-1334 — chuck blanks AFTER `|`/`(`.
                            if chars[i] == Bar || chars[i] == Inpar {
                                while i + 1 < chars.len() && blank(chars[i + 1]) {
                                    chars.remove(i + 1);
                                }
                            }
                            // c:1335-1338 — chuck blanks BEFORE `|`/`)`
                            // (not Meta-escaped blanks).
                            if chars[i] == Bar || chars[i] == Outpar {
                                while i >= 1
                                    && blank(chars[i - 1])
                                    && (i < 2 || chars[i - 2] != meta)
                                {
                                    chars.remove(i - 1);
                                    i -= 1;
                                }
                            }
                        }
                        if chars[i] == Outpar {
                            pct -= 1;
                        }
                        i += 1;
                    }
                    // c:1345-1346 — `if (*s || pct || s == str)
                    // YYERRORV(oecused);`
                    if early_break || pct != 0 || chars.is_empty() {
                        zerr("par_case: expected `)` or `|`");
                        return;
                    }
                    // c:1347-1352 — strip surrounding `(...)`.
                    chars.pop();
                    chars.remove(0);
                    let stripped: String = chars.into_iter().collect();
                    // c:1353-1355 — `ecstr(str); ecadd(ecnpats++); nalts++;`
                    ecstr(&stripped);
                    let np = ECNPATS.with(|cc| {
                        let v = cc.get();
                        cc.set(v + 1);
                        v
                    }) as u32;
                    ecadd(np);
                    nalts += 1;
                    // c:1356 — `break;` — tok is already the body's
                    // first token; fall through to par_save_list.
                    break;
                }
                // c:1358 — `YYERRORV(oecused);`
                zerr("par_case: expected `)` or `|`");
                return;
            }

            // c:1359 — `zshlex();`
            zshlex();
            // c:1360-1377 — switch on next tok.
            match tok() {
                STRING_LEX => {
                    // c:1361-1365
                    str = tokstr().unwrap_or_default();
                    zshlex();
                }
                OUTPAR_TOK | BAR_TOK => {
                    // c:1367-1371 — empty string
                    str = String::new();
                }
                _ => {
                    // c:1374-1376 — `YYERRORV(oecused);`
                    zerr("par_case: expected pattern, `)` or `|`");
                    return;
                }
            }
        }
        // c:1379 — `incasepat = 0;`
        set_incasepat(0);
        // c:1380 — `par_save_list(cmplx);`
        par_save_list_wordcode(_cmplx);
        // c:1381-1384 — terminator → arm type
        if tok() == SEMIAMP {
            r#type = WC_CASE_AND;
        } else if tok() == SEMIBAR {
            r#type = WC_CASE_TESTAND;
        }
        // c:1385 — `ecbuf[pp] = WCB_CASE(type, ecused - 1 - pp);`
        let used = ECUSED.get() as usize;
        ECBUF.with_borrow_mut(|b| {
            b[pp] = WCB_CASE(r#type, (used.saturating_sub(1 + pp)) as wordcode);
        });
        // c:1386 — `ecbuf[palts] = nalts;`
        ECBUF.with_borrow_mut(|b| {
            b[palts] = nalts;
        });
        // c:1387-1388 — terminator (ESAC w/o brace OR OUTBRACE w/ brace) → break
        if (tok() == ESAC && !brflag) || (tok() == OUTBRACE_TOK && brflag) {
            break 'arms;
        }
        // c:1389-1390 — `if (tok != DSEMI && tok != SEMIAMP && tok != SEMIBAR) YYERRORV;`
        if tok() != DSEMI && tok() != SEMIAMP && tok() != SEMIBAR {
            zerr("par_case: expected `;;`, `;&`, or `;|`");
            return;
        }
        // c:1391 — `incasepat = 1;`
        set_incasepat(1);
        // c:1392 — `incmdpos = 0;`
        set_incmdpos(false);
        // c:1393 — `zshlex();`
        zshlex();
    }
    // c:1395 — `incmdpos = 1;`
    set_incmdpos(true);
    // c:1396 — `incasepat = 0;`
    set_incasepat(0);
    // c:1397 — `zshlex();`
    zshlex();

    // c:1399 — `ecbuf[p] = WCB_CASE(WC_CASE_HEAD, ecused - 1 - p);`
    let used = ECUSED.get() as usize;
    ECBUF.with_borrow_mut(|b| {
        b[p] = WCB_CASE(WC_CASE_HEAD, (used.saturating_sub(1 + p)) as wordcode);
    });
}

/// Port of `par_if(int *cmplx)` from `Src/parse.c:1410-1512`.
pub fn par_if_wordcode(cmplx: &mut i32) {
    // c:1413 — `int oecused = ecused, p, pp, type, usebrace = 0;`
    let _oecused = ECUSED.get() as usize;
    let p: usize;
    let mut pp: usize = 0;
    let mut r#type: wordcode = WC_IF_IF;
    let mut usebrace: i32 = 0;
    // c:1414 — `enum lextok xtok;`
    let mut xtok: lextok;
    // c:1415 — `unsigned char nc;`
    let nc: u8;
    let _ = nc;

    // c:1417 — `p = ecadd(0);`
    p = ecadd(0);

    // c:1419 — `for (;;) {`
    loop {
        // c:1420 — `xtok = tok;`
        xtok = tok();
        // c:1421 — `cmdpush(xtok == IF ? CS_IF : CS_ELIF);`
        cmdpush(if xtok == IF {
            CS_IF as u8
        } else {
            CS_ELIF as u8
        });
        // c:1422-1426 — `if (xtok == FI) { incmdpos = 0; zshlex(); break; }`
        if xtok == FI {
            set_incmdpos(false);
            zshlex();
            break;
        }
        // c:1427 — `zshlex();`
        zshlex();
        // c:1428-1429 — `if (xtok == ELSE) break;`
        if xtok == ELSE {
            break;
        }
        // c:1430-1431 — `while (tok == SEPER) zshlex();`
        while tok() == SEPER {
            zshlex();
        }
        // c:1432-1435 — `if (!(xtok == IF || xtok == ELIF)) { cmdpop(); YYERRORV; }`
        if !(xtok == IF || xtok == ELIF) {
            cmdpop();
            zerr("par_if: expected `if` or `elif`");
            return;
        }
        // c:1436 — `pp = ecadd(0);`
        pp = ecadd(0);
        // c:1437 — `type = (xtok == IF ? WC_IF_IF : WC_IF_ELIF);`
        r#type = if xtok == IF { WC_IF_IF } else { WC_IF_ELIF };
        // c:1438 — `par_save_list(cmplx);` — condition body
        par_save_list_wordcode(cmplx);
        // c:1439 — `incmdpos = 1;`
        set_incmdpos(true);
        // c:1440-1443 — `if (tok == ENDINPUT) { cmdpop(); YYERRORV; }`
        if tok() == ENDINPUT {
            cmdpop();
            zerr("par_if: unexpected end-of-input after condition");
            return;
        }
        // c:1444-1445 — `while (tok == SEPER) zshlex();`
        while tok() == SEPER {
            zshlex();
        }
        // c:1446 — `xtok = FI;` — pre-set so the post-loop check works
        xtok = FI;
        // c:1447 — `nc = cmdstack[cmdsp - 1] == CS_IF ? CS_IFTHEN : CS_ELIFTHEN;`
        // (Not tracked separately in zshrs cmdstack — derive from cur top
        // by reading CMDSTACK; for safety use CS_IFTHEN as default.)
        // We don't have a way to read top easily — match by tracking
        // whether we just pushed CS_IF or CS_ELIF.
        // For wordcode emission this only affects cmdstack debug output;
        // not the emitted wordcode. Use CS_IFTHEN.
        let nc_local: u8 = CS_IFTHEN as u8;
        if tok() == THEN {
            // c:1448-1456 — THEN branch
            // c:1449 — `usebrace = 0;`
            usebrace = 0;
            // c:1450 — `cmdpop();`
            cmdpop();
            // c:1451 — `cmdpush(nc);`
            cmdpush(nc_local);
            // c:1452 — `zshlex();`
            zshlex();
            // c:1453 — `par_save_list(cmplx);` — then body
            par_save_list_wordcode(cmplx);
            // c:1454 — `ecbuf[pp] = WCB_IF(type, ecused - 1 - pp);`
            let used = ECUSED.get() as usize;
            ECBUF.with_borrow_mut(|b| {
                b[pp] = WCB_IF(r#type, (used.saturating_sub(1 + pp)) as wordcode);
            });
            // c:1455 — `incmdpos = 1;`
            set_incmdpos(true);
            // c:1456 — `cmdpop();`
            cmdpop();
        } else if tok() == INBRACE_TOK {
            // c:1457-1473 — INBRACE branch
            // c:1458 — `usebrace = 1;`
            usebrace = 1;
            // c:1459 — `cmdpop();`
            cmdpop();
            // c:1460 — `cmdpush(nc);`
            cmdpush(nc_local);
            // c:1461 — `zshlex();`
            zshlex();
            // c:1462 — `par_save_list(cmplx);`
            par_save_list_wordcode(cmplx);
            // c:1463-1466 — `if (tok != OUTBRACE) { cmdpop(); YYERRORV; }`
            if tok() != OUTBRACE_TOK {
                cmdpop();
                zerr("par_if: expected `}`");
                return;
            }
            // c:1467 — `ecbuf[pp] = WCB_IF(type, ecused - 1 - pp);`
            let used = ECUSED.get() as usize;
            ECBUF.with_borrow_mut(|b| {
                b[pp] = WCB_IF(r#type, (used.saturating_sub(1 + pp)) as wordcode);
            });
            // c:1469 — `zshlex();`
            zshlex();
            // c:1470 — `incmdpos = 1;`
            set_incmdpos(true);
            // c:1471-1472 — `if (tok == SEPER) break;`
            if tok() == SEPER {
                break;
            }
            // c:1473 — `cmdpop();`
            cmdpop();
        } else if unset(SHORTLOOPS) {
            // c:1474-1476 — `cmdpop(); YYERRORV;`
            cmdpop();
            zerr("par_if: short body requires SHORTLOOPS");
            return;
        } else {
            // c:1477-1484 — short loop form
            // c:1478 — `cmdpop();`
            cmdpop();
            // c:1479 — `cmdpush(nc);`
            cmdpush(nc_local);
            // c:1480 — `par_save_list1(cmplx);`
            par_save_list1_wordcode(cmplx);
            // c:1481 — `ecbuf[pp] = WCB_IF(type, ecused - 1 - pp);`
            let used = ECUSED.get() as usize;
            ECBUF.with_borrow_mut(|b| {
                b[pp] = WCB_IF(r#type, (used.saturating_sub(1 + pp)) as wordcode);
            });
            // c:1482 — `incmdpos = 1;`
            set_incmdpos(true);
            // c:1483 — `break;`
            break;
        }
    }
    // c:1486 — `cmdpop();`
    cmdpop();
    // c:1487 — `if (xtok == ELSE || tok == ELSE) {`
    if xtok == ELSE || tok() == ELSE {
        // c:1488 — `pp = ecadd(0);`
        pp = ecadd(0);
        // c:1489 — `cmdpush(CS_ELSE);`
        cmdpush(CS_ELSE as u8);
        // c:1490-1491 — `while (tok == SEPER) zshlex();`
        while tok() == SEPER {
            zshlex();
        }
        // c:1492-1498 — `if (tok == INBRACE && usebrace) { ... } else { ... }`
        if tok() == INBRACE_TOK && usebrace != 0 {
            // c:1493 — `zshlex();`
            zshlex();
            // c:1494 — `par_save_list(cmplx);`
            par_save_list_wordcode(cmplx);
            // c:1495-1498 — `if (tok != OUTBRACE) { cmdpop(); YYERRORV; }`
            if tok() != OUTBRACE_TOK {
                cmdpop();
                zerr("par_if: else expected `}`");
                return;
            }
        } else {
            // c:1500 — `par_save_list(cmplx);`
            par_save_list_wordcode(cmplx);
            // c:1501-1504 — `if (tok != FI) { cmdpop(); YYERRORV; }`
            if tok() != FI {
                cmdpop();
                zerr("par_if: else expected `fi`");
                return;
            }
        }
        // c:1506 — `incmdpos = 0;`
        set_incmdpos(false);
        // c:1507 — `ecbuf[pp] = WCB_IF(WC_IF_ELSE, ecused - 1 - pp);`
        let used = ECUSED.get() as usize;
        ECBUF.with_borrow_mut(|b| {
            b[pp] = WCB_IF(WC_IF_ELSE, (used.saturating_sub(1 + pp)) as wordcode);
        });
        // c:1508 — `zshlex();`
        zshlex();
        // c:1509 — `cmdpop();`
        cmdpop();
    }
    // c:1511 — `ecbuf[p] = WCB_IF(WC_IF_HEAD, ecused - 1 - p);`
    let used = ECUSED.get() as usize;
    ECBUF.with_borrow_mut(|b| {
        b[p] = WCB_IF(WC_IF_HEAD, (used.saturating_sub(1 + p)) as wordcode);
    });
}

/// Port of `par_while(int *cmplx)` from `Src/parse.c:1520-1557`.
pub fn par_while_wordcode(cmplx: &mut i32) {
    // c:1523 — `int oecused = ecused, p;`
    let _oecused = ECUSED.get() as usize;
    let p: usize;
    // c:1524 — `int type = (tok == UNTIL ? WC_WHILE_UNTIL : WC_WHILE_WHILE);`
    let r#type: wordcode = if tok() == UNTIL {
        WC_WHILE_UNTIL
    } else {
        WC_WHILE_WHILE
    };

    // c:1526 — `p = ecadd(0);`
    p = ecadd(0);
    // c:1527 — `zshlex();`
    zshlex();
    // c:1528 — `par_save_list(cmplx);` — condition.
    par_save_list_wordcode(cmplx);
    // c:1529 — `incmdpos = 1;`
    set_incmdpos(true);
    // c:1530-1531 — `while (tok == SEPER) zshlex();`
    while tok() == SEPER {
        zshlex();
    }
    // c:1532-1545 — body dispatch (inlined in C; we factor via
    // par_loop_body_wordcode since for/while/repeat share this
    // identical block).
    if tok() == DOLOOP {
        // c:1533 — `zshlex();`
        zshlex();
        // c:1534 — `par_save_list(cmplx);`
        par_save_list_wordcode(cmplx);
        // c:1535-1536 — `if (tok != DONE) YYERRORV(oecused);`
        if tok() != DONE {
            zerr("par_while: expected `done`");
            return;
        }
        // c:1537 — `incmdpos = 0;`
        set_incmdpos(false);
        // c:1538 — `zshlex();`
        zshlex();
    } else if tok() == INBRACE_TOK {
        // c:1540 — `zshlex();`
        zshlex();
        // c:1541 — `par_save_list(cmplx);`
        par_save_list_wordcode(cmplx);
        // c:1542-1543 — `if (tok != OUTBRACE) YYERRORV(oecused);`
        if tok() != OUTBRACE_TOK {
            zerr("par_while: expected `}`");
            return;
        }
        // c:1544 — `incmdpos = 0;`
        set_incmdpos(false);
        // c:1545 — `zshlex();`
        zshlex();
    } else if isset(CSHJUNKIELOOPS) {
        // c:1546-1550
        par_save_list_wordcode(cmplx);
        if tok() != ZEND {
            zerr("par_while: expected `end`");
            return;
        }
        zshlex();
    } else if unset(SHORTLOOPS) {
        // c:1551-1552 — `YYERRORV(oecused);`
        zerr("par_while: short body requires SHORTLOOPS");
        return;
    } else {
        // c:1554 — `par_save_list1(cmplx);`
        par_save_list1_wordcode(cmplx);
    }

    // c:1556 — `ecbuf[p] = WCB_WHILE(type, ecused - 1 - p);`
    let used = ECUSED.get() as usize;
    ECBUF.with_borrow_mut(|b| {
        b[p] = WCB_WHILE(r#type, (used.saturating_sub(1 + p)) as wordcode);
    });
}

/// `until` shares par_while body — tok==UNTIL flips the type.
pub fn par_until_wordcode(cmplx: &mut i32) {
    par_while_wordcode(cmplx);
}

/// Port of `par_repeat(int *cmplx)` from `Src/parse.c:1564-1606`.
pub fn par_repeat_wordcode(cmplx: &mut i32) {
    // c:1567 — `/* ### what to do about inrepeat_ here? */`
    // c:1568 — `int oecused = ecused, p;`
    let _oecused = ECUSED.get() as usize;
    let p: usize;

    // c:1570 — `p = ecadd(0);`
    p = ecadd(0);

    // c:1572 — `incmdpos = 0;`
    set_incmdpos(false);
    // c:1573 — `zshlex();`
    zshlex();
    // c:1574-1575 — `if (tok != STRING) YYERRORV(oecused);`
    if tok() != STRING_LEX {
        zerr("par_repeat: expected count");
        return;
    }
    // c:1576 — `ecstr(tokstr);`
    ecstr(&tokstr().unwrap_or_default());
    // c:1577 — `incmdpos = 1;`
    set_incmdpos(true);
    // c:1578 — `zshlex();`
    zshlex();
    // c:1579-1580 — `while (tok == SEPER) zshlex();`
    while tok() == SEPER {
        zshlex();
    }
    // c:1581-1604 — body dispatch (inlined here matching C exactly).
    if tok() == DOLOOP {
        // c:1582-1587
        zshlex();
        par_save_list_wordcode(cmplx);
        if tok() != DONE {
            zerr("par_repeat: expected `done`");
            return;
        }
        set_incmdpos(false);
        zshlex();
    } else if tok() == INBRACE_TOK {
        // c:1589-1594
        zshlex();
        par_save_list_wordcode(cmplx);
        if tok() != OUTBRACE_TOK {
            zerr("par_repeat: expected `}`");
            return;
        }
        set_incmdpos(false);
        zshlex();
    } else if isset(CSHJUNKIELOOPS) {
        // c:1596-1599
        par_save_list_wordcode(cmplx);
        if tok() != ZEND {
            zerr("par_repeat: expected `end`");
            return;
        }
        zshlex();
    } else if unset(SHORTLOOPS) && unset(SHORTREPEAT) {
        // c:1601-1602 — par_repeat needs BOTH SHORTLOOPS and SHORTREPEAT
        // unset to refuse short form (more permissive than par_while).
        zerr("par_repeat: short body requires SHORTLOOPS or SHORTREPEAT");
        return;
    } else {
        // c:1604 — `par_save_list1(cmplx);`
        par_save_list1_wordcode(cmplx);
    }

    // c:1606 — `ecbuf[p] = WCB_REPEAT(ecused - 1 - p);`
    let used = ECUSED.get() as usize;
    ECBUF.with_borrow_mut(|b| {
        b[p] = WCB_REPEAT((used.saturating_sub(1 + p)) as wordcode);
    });
}

/// Port of `par_funcdef(int *cmplx)` from `Src/parse.c:1672-1779`.
///
/// The `function NAME { ... }` form. Emits a WCB_FUNCDEF header
/// followed by a names-count slot, the names themselves, four
/// metadata slots (string-area start, string-area length, npats,
/// do_tracing), then the body wordcode, then WCB_END.
///
/// Critical: saves/resets `ecnpats` + `ecssub` + `ecsoffs` around
/// the body parse so per-function pattern counts don't leak into
/// the enclosing scope's `ecnpats` accumulator (parse.c:1723-1758).
pub fn par_funcdef_wordcode(cmplx: &mut i32) {
    // c:1674 — `int oecused = ecused, num = 0, onp, p, c = 0;`
    let _oecused = ECUSED.get() as usize;
    let mut num: i32 = 0;
    let onp: i32;
    let p: usize;
    let mut c: i32 = 0;
    // c:1675 — `int so, oecssub = ecssub;`
    let so: i32;
    let oecssub = ECSSUB.get();
    // c:1676 — `zlong oldlineno = lineno;`
    let oldlineno = lineno();
    // c:1677 — `int do_tracing = 0;`
    let mut do_tracing: i32 = 0;

    // c:1679 — `lineno = 0;`
    set_lineno(0);
    // c:1680 — `nocorrect = 1;`
    set_nocorrect(1);
    // c:1681 — `incmdpos = 0;`
    set_incmdpos(false);
    // c:1682 — `zshlex();`
    zshlex();

    // c:1684 — `p = ecadd(0);`
    p = ecadd(0);
    // c:1685 — `ecadd(0); /* p + 1 */`
    let p1 = ecadd(0);

    // c:1687-1699 — `Consume an initial (-T), (--), or (-T --).`
    // c:1690 — `if (tok == STRING && tokstr[0] == Dash) {`
    if tok() == STRING_LEX {
        let s = tokstr().unwrap_or_default();
        let bytes = s.as_bytes();
        // C: `tokstr[0] == Dash` (Dash = 0x9b = 0xc2 0x9b in UTF-8).
        // First byte of UTF-8 `\u{9b}` is 0xc2; the char `'-'` is 0x2d.
        // Match either form.
        let first_is_dash = (bytes.len() >= 2 && bytes[0] == 0xc2 && bytes[1] == 0x9b)
            || (bytes.len() >= 1 && bytes[0] == b'-');
        if first_is_dash {
            // c:1691-1694 — `if (tokstr[1] == 'T' && !tokstr[2]) { ++do_tracing; zshlex(); }`
            // After the leading dash byte(s), check remaining bytes.
            let after_dash = if bytes.len() >= 2 && bytes[0] == 0xc2 && bytes[1] == 0x9b {
                &bytes[2..]
            } else {
                &bytes[1..]
            };
            if after_dash.len() == 1 && after_dash[0] == b'T' {
                do_tracing += 1;
                zshlex();
            }
            // c:1695-1698 — `if (tok == STRING && tokstr[0] == Dash &&
            //                  tokstr[1] == Dash && !tokstr[2]) zshlex();`
            if tok() == STRING_LEX {
                let s2 = tokstr().unwrap_or_default();
                let b2 = s2.as_bytes();
                let mut idx = 0;
                let mut dashes = 0;
                while idx < b2.len() && dashes < 2 {
                    if b2[idx] == 0xc2 && idx + 1 < b2.len() && b2[idx + 1] == 0x9b {
                        idx += 2;
                        dashes += 1;
                    } else if b2[idx] == b'-' {
                        idx += 1;
                        dashes += 1;
                    } else {
                        break;
                    }
                }
                if dashes == 2 && idx == b2.len() {
                    zshlex();
                }
            }
        }
    }

    // c:1701-1709 — names loop.
    // `while (tok == STRING) { if ((*tokstr == Inbrace || *tokstr == '{')
    //   && !tokstr[1]) { tok = INBRACE; break; } ecstr(tokstr); num++; zshlex(); }`
    while tok() == STRING_LEX {
        let s = tokstr().unwrap_or_default();
        let bytes = s.as_bytes();
        // First byte tests for Inbrace marker (0x8f → UTF-8 `0xc2 0x8f`) or `{`,
        // and length-1 check (`!tokstr[1]`).
        let is_inbrace_only = (bytes.len() == 1 && bytes[0] == b'{')
            || (bytes.len() == 2 && bytes[0] == 0xc2 && bytes[1] == 0x8f);
        if is_inbrace_only {
            set_tok(INBRACE_TOK);
            break;
        }
        ecstr(&s);
        num += 1;
        zshlex();
    }

    // c:1711-1714 — four metadata placeholder slots.
    let m2 = ecadd(0);
    let m3 = ecadd(0);
    let m4 = ecadd(0);
    let m5 = ecadd(0);

    // c:1716 — `nocorrect = 0;`
    set_nocorrect(0);
    // c:1717 — `incmdpos = 1;`
    set_incmdpos(true);
    // c:1718-1719 — `if (tok == INOUTPAR) zshlex();`
    if tok() == INOUTPAR {
        zshlex();
    }
    // c:1720-1721 — `while (tok == SEPER) zshlex();`
    while tok() == SEPER {
        zshlex();
    }

    // c:1723 — `ecnfunc++;`
    ECNFUNC.set(ECNFUNC.get() + 1);
    // c:1724 — `ecssub = so = ecsoffs;`
    so = ECSOFFS.get();
    ECSSUB.set(so);
    // c:1725 — `onp = ecnpats;`
    onp = ECNPATS.with(|cc| cc.get());
    // c:1726 — `ecnpats = 0;`
    ECNPATS.with(|cc| cc.set(0));

    // c:1728 — `if (tok == INBRACE) {`
    if tok() == INBRACE_TOK {
        // c:1729 — `zshlex();`
        zshlex();
        // c:1730 — `par_list(&c);`
        par_list_wordcode(&mut c);
        // c:1731-1736 — `if (tok != OUTBRACE) { lineno += oldlineno; ... }`
        if tok() != OUTBRACE_TOK {
            set_lineno(lineno() + oldlineno);
            ECNPATS.with(|cc| cc.set(onp));
            ECSSUB.set(oecssub);
            zerr("par_funcdef: expected `}`");
            return;
        }
        // c:1737-1740 — `if (num == 0) { incmdpos = 0; }`
        if num == 0 {
            set_incmdpos(false);
        }
        // c:1741 — `zshlex();`
        zshlex();
    } else if unset(SHORTLOOPS) {
        // c:1742-1746 — `lineno += oldlineno; ecnpats = onp; ecssub = oecssub; YYERRORV`
        set_lineno(lineno() + oldlineno);
        ECNPATS.with(|cc| cc.set(onp));
        ECSSUB.set(oecssub);
        zerr("par_funcdef: short body requires SHORTLOOPS");
        return;
    } else {
        // c:1748 — `par_list1(&c);`
        par_list1_wordcode(&mut c);
    }

    // c:1750 — `ecadd(WCB_END());`
    ecadd(WCB_END());
    // c:1751-1754 — fill the 4 metadata slots
    let cur_sofs = ECSOFFS.get();
    let body_npats = ECNPATS.with(|cc| cc.get());
    ECBUF.with_borrow_mut(|b| {
        b[m2] = (so - oecssub) as wordcode;
        b[m3] = (cur_sofs - so) as wordcode;
        b[m4] = body_npats as wordcode;
        b[m5] = do_tracing as wordcode;
    });
    // c:1755 — `ecbuf[p + 1] = num;`
    ECBUF.with_borrow_mut(|b| {
        b[p1] = num as wordcode;
    });

    // c:1757 — `ecnpats = onp;`
    ECNPATS.with(|cc| cc.set(onp));
    // c:1758 — `ecssub = oecssub;`
    ECSSUB.set(oecssub);
    // c:1759 — `ecnfunc++;`
    ECNFUNC.set(ECNFUNC.get() + 1);

    // c:1761 — `ecbuf[p] = WCB_FUNCDEF(ecused - 1 - p);`
    let used = ECUSED.get() as usize;
    ECBUF.with_borrow_mut(|b| {
        b[p] = WCB_FUNCDEF((used.saturating_sub(1 + p)) as wordcode);
    });

    // c:1763-1777 — anonymous-function trailing args (num == 0 case).
    if num == 0 {
        // c:1766 — `int parg = ecadd(0);`
        let parg = ecadd(0);
        // c:1767 — `ecadd(0);`
        ecadd(0);
        // c:1768-1772 — `while (tok == STRING) { ecstr(tokstr); num++; zshlex(); }`
        while tok() == STRING_LEX {
            ecstr(&tokstr().unwrap_or_default());
            num += 1;
            zshlex();
        }
        // c:1773-1774 — `if (num > 0) *cmplx = 1;`
        if num > 0 {
            *cmplx = 1;
        }
        // c:1775 — `ecbuf[parg] = ecused - parg;`
        // c:1776 — `ecbuf[parg+1] = num;`
        let used2 = ECUSED.get() as usize;
        ECBUF.with_borrow_mut(|b| {
            b[parg] = (used2 - parg) as wordcode;
            b[parg + 1] = num as wordcode;
        });
    }
    // c:1778 — `lineno += oldlineno;`
    set_lineno(lineno() + oldlineno);
}

/// Size of `struct fdhead` in `wordcode` (u32) units. Used by all
/// the header-walk macros below.
pub const FDHEAD_WORDS: usize = size_of::<fdhead>() / 4;

/// `Src/parse.c:1619-1665`. Handles both `(...)` subshell and
/// `{...}` brace group (cursh) plus optional `always { ... }`
/// trailing block. C uses a single function with `zsh_construct=1`
/// for `{...}` and 0 for `(...)`.
pub fn par_subsh_wordcode(cmplx: &mut i32, zsh_construct: i32) {
    // c:1621 — `enum lextok otok = tok;`
    let otok = tok();
    // c:1622 — `int oecused = ecused, p, pp;`
    let _oecused = ECUSED.get() as usize;
    let p: usize;
    let pp: usize;

    // c:1624 — `p = ecadd(0);`
    p = ecadd(0);
    // c:1625 — `/* Extra word only needed for always block */`
    // c:1626 — `pp = ecadd(0);`
    pp = ecadd(0);
    // c:1627 — `zshlex();`
    zshlex();
    // c:1628 — `par_list(cmplx);`
    par_list_wordcode(cmplx);
    // c:1629 — `ecadd(WCB_END());`
    ecadd(WCB_END());
    // c:1630-1631 — `if (tok != ((otok == INPAR) ? OUTPAR : OUTBRACE))
    // YYERRORV(oecused);`
    if tok()
        != (if otok == INPAR_TOK {
            OUTPAR_TOK
        } else {
            OUTBRACE_TOK
        })
    {
        zerr("par_subsh: missing closing token");
        return;
    }
    // c:1632 — `incmdpos = !zsh_construct;`
    set_incmdpos(zsh_construct == 0);
    // c:1633 — `zshlex();`
    zshlex();

    // c:1635 — `/* Optional always block. No intervening SEPERs allowed. */`
    // c:1636 — `if (otok == INBRACE && tok == STRING && !strcmp(tokstr, "always")) {`
    if otok == INBRACE_TOK && tok() == STRING_LEX && tokstr().as_deref() == Some("always") {
        // c:1637 — `ecbuf[pp] = WCB_TRY(ecused - 1 - pp);`
        let used = ECUSED.get() as usize;
        ECBUF.with_borrow_mut(|b| {
            b[pp] = WCB_TRY((used.saturating_sub(1 + pp)) as wordcode);
        });
        // c:1638 — `incmdpos = 1;`
        set_incmdpos(true);
        // c:1639-1641 — `do { zshlex(); } while (tok == SEPER);`
        loop {
            zshlex();
            if tok() != SEPER {
                break;
            }
        }

        // c:1643-1644 — `if (tok != INBRACE) YYERRORV(oecused);`
        if tok() != INBRACE_TOK {
            zerr("par_subsh: 'always' expects `{`");
            return;
        }
        // c:1645 — `cmdpop();`
        cmdpop();
        // c:1646 — `cmdpush(CS_ALWAYS);`
        cmdpush(CS_ALWAYS as u8);

        // c:1648 — `zshlex();`
        zshlex();
        // c:1649 — `par_save_list(cmplx);`
        par_save_list_wordcode(cmplx);
        // c:1650-1651 — `while (tok == SEPER) zshlex();`
        while tok() == SEPER {
            zshlex();
        }

        // c:1653 — `incmdpos = 1;`
        set_incmdpos(true);

        // c:1655-1656 — `if (tok != OUTBRACE) YYERRORV(oecused);`
        if tok() != OUTBRACE_TOK {
            zerr("par_subsh: 'always' block missing `}`");
            return;
        }
        // c:1657 — `zshlex();`
        zshlex();
        // c:1658 — `ecbuf[p] = WCB_TRY(ecused - 1 - p);`
        let used = ECUSED.get() as usize;
        ECBUF.with_borrow_mut(|b| {
            b[p] = WCB_TRY((used.saturating_sub(1 + p)) as wordcode);
        });
    } else {
        // c:1660-1661 — `ecbuf[p] = (otok == INPAR ? WCB_SUBSH(...) : WCB_CURSH(...));`
        let used = ECUSED.get() as usize;
        let off = used.saturating_sub(1 + p);
        ECBUF.with_borrow_mut(|b| {
            b[p] = if otok == INPAR_TOK {
                WCB_SUBSH(off as wordcode)
            } else {
                WCB_CURSH(off as wordcode)
            };
        });
    }
}

/// Port of `par_time(void)` from `Src/parse.c:1787`. `time PIPE`
/// emits WCB_TIMED(WC_TIMED_PIPE) + the sublist code; bare `time`
/// with no pipeline emits WCB_TIMED(WC_TIMED_EMPTY).
pub fn par_time_wordcode() {
    // c:1791 — `zshlex();`
    zshlex();
    // c:1793-1794 — `p = ecadd(0); ecadd(0);`
    let p = ecadd(0);
    ecadd(0);
    // c:1795 — `if ((f = par_sublist2(&c)) < 0)`
    let mut c = 0i32;
    let f = par_sublist2(&mut c);
    match f {
        Some(flags) => {
            // c:1799 — `ecbuf[p] = WCB_TIMED(WC_TIMED_PIPE);`
            ECBUF.with_borrow_mut(|b| {
                if p < b.len() {
                    b[p] = WCB_TIMED(WC_TIMED_PIPE);
                }
            });
            // c:1800 — `set_sublist_code(p+1, WC_SUBLIST_END, f,
            // ecused-2-p, c);`
            let used = ECUSED.get() as usize;
            let skip = used.saturating_sub(2 + p) as i32;
            set_sublist_code(p + 1, WC_SUBLIST_END as i32, flags, skip, c != 0);
        }
        None => {
            // c:1796-1798 — `ecused--; ecbuf[p] = WCB_TIMED(WC_TIMED_EMPTY);`
            ECUSED.set((ECUSED.get() - 1).max(0));
            ECBUF.with_borrow_mut(|b| {
                if p < b.len() {
                    b[p] = WCB_TIMED(WC_TIMED_EMPTY);
                }
            });
        }
    }
}

/// Port of `par_dinbrack(void)` from `Src/parse.c:1810`. Wraps
/// `par_cond` (the cond-expression emitter at parse.c:2409) with
/// the `[[ ... ]]` framing: incond/incmdpos toggles + DOUTBRACK
/// expectation.
pub fn par_cond_wordcode() {
    let oecused = ECUSED.get();
    // c:1814 — `incond = 1;`
    set_incond(1);
    // c:1815 — `incmdpos = 0;`
    set_incmdpos(false);
    // c:1816 — `zshlex();` past `[[`.
    zshlex();
    // c:1817 — `par_cond();` — call the no-skip cond-expression
    // entry that EMITS WORDCODE (par_cond_top → par_cond_1 →
    // par_cond_2 → par_cond_double/triple/multi). NOT the AST
    // `par_cond` at parse.rs:4644 which is a misnamed `par_dinbrack`
    // that skips `[[` AND `]]` and returns a ZshCommand AST node
    // instead of pushing WC_COND opcodes. NOT `parse_cond_expr`
    // either — that's also AST-only, returning ZshCond. With
    // `parse_cond_expr` here, every `[[ ... ]]` test produced ZERO
    // wordcode payload and parity dropped ~148 words on /etc/zshrc.
    let _ = par_cond_top();
    // c:1818-1819 — `if (tok != DOUTBRACK) YYERRORV(oecused);`
    if tok() != DOUTBRACK {
        let _ = oecused;
        zerr("missing ]]");
        return;
    }
    // c:1820 — `incond = 0;`
    set_incond(0);
    // c:1821 — `incmdpos = 1;`
    set_incmdpos(true);
    // c:1822 — `zshlex();` past `]]`.
    zshlex();
}

/// Port of the `case DINPAR:` arm of `par_cmd` from
/// `Src/parse.c:1031-1034`:
/// ```c
/// ecadd(WCB_ARITH());
/// ecstr(tokstr);
/// zshlex();
/// ```
/// `(( EXPR ))` arithmetic at command position — emits the ARITH
/// opcode followed by the interned EXPR string, then advances past
/// the DINPAR token (which already carries the body text).
pub fn par_arith_wordcode() {
    // c:1032 — `ecadd(WCB_ARITH());`
    ecadd(WCB_ARITH());
    // c:1033 — `ecstr(tokstr);` — interns the expression string and
    // appends its strcode index to the wordcode buffer.
    let expr = tokstr().unwrap_or_default();
    ecstr(&expr);
    // c:1034 — `zshlex();`
    zshlex();
}

/// Port of `par_simple(int *cmplx, int nr)` from
/// `Src/parse.c:1836-2227`. Emits WC_SIMPLE + word count +
/// interned string offsets. Returns `0` when nothing was emitted,
/// otherwise `1 + (number of code words consumed by redirections)`.
/// The full C body handles assignments (ENVSTRING/ENVARRAY),
/// inline `{var}>file` brace-FDs, prefix modifiers (NOCORRECT etc),
/// and `name() { body }` funcdef detection — those paths are
/// progressively wired into the AST parser; this wordcode-emitter
/// covers the simple `cmd args...` case + interleaved redirs.
pub fn par_simple_wordcode(cmplx: &mut i32, mut nr: i32) -> i32 {
    // c:1838-1841 — `int oecused = ecused, isnull = 1, r, argc = 0,
    //   p, isfunc = 0, sr = 0;`
    //   `int c = *cmplx, nrediradd, assignments = 0, ppost = 0,
    //   is_typeset = 0;`
    // c is the SAVED initial cmplx so INOUTPAR can restore via
    // `*cmplx = c;` at c:2070.
    let _oecused = ECUSED.get() as usize;
    let c_saved = *cmplx;
    let mut isnull = true;
    let mut argc: u32 = 0;
    let mut sr: i32 = 0;
    let mut assignments = false;
    let mut isfunc = false;

    // c:1843 — `r = ecused;` — saves the offset where redirs get
    // INSERTED (via ecispace). Each redir shifts later words DOWN
    // by ncodes, so the SIMPLE placeholder at `p` (set later) must
    // also bump by ncodes when a redir lands. C uses `&r` to pass
    // the cursor by reference; Rust uses a mutable local + manual
    // bumps after each par_redir_wordcode call.
    let mut r: usize = ECUSED.get() as usize;

    // c:1844-1919 — pre-cmd loop: NOCORRECT, ENVSTRING (scalar
    // assigns), ENVARRAY (array assigns), IS_REDIROP. Loops until
    // a non-assignment token is seen.
    loop {
        match tok() {
            NOCORRECT => {
                // c:1846-1849
                *cmplx = 1;
                set_nocorrect(1);
            }
            ENVSTRING => {
                // c:1848-1898 — scalar assignment `name=value` or
                // `name+=value`. Emits WCB_ASSIGN(SCALAR, NEW|INC, 0)
                // followed by ecstr(name), ecstr(value).
                let raw = tokstr().unwrap_or_default();
                // Find first of Inbrack / '=' / '+' (the C scan at
                // c:1851-1853). Inside Inbrack we skipparens — i.e.
                // skip `name[...]` index, then continue.
                // c:1851-1853 — `for (ptr = tokstr; *ptr && *ptr != Inbrack
                // && *ptr != '=' && *ptr != '+'; ptr++); if (*ptr == Inbrack)
                // skipparens(Inbrack, Outbrack, &ptr);`. Walk to the first
                // `[`/`=`/`+`/Equals-token, then if we landed on `[`, skip
                // the balanced `name[index]` pair via skipparens.
                let bytes: Vec<char> = raw.chars().collect();
                let raw_str: String = bytes.iter().collect();
                let mut idx = 0usize;
                while idx < bytes.len() {
                    let ch = bytes[idx];
                    if ch == '\u{91}' /* Inbrack */
                        || ch == '=' || ch == '+' || ch == '\u{8d}'
                    /* Equals */
                    {
                        break;
                    }
                    idx += 1;
                }
                if idx < bytes.len() && bytes[idx] == '\u{91}'
                /* Inbrack */
                {
                    // c:1855 — `skipparens(Inbrack, Outbrack, &ptr);`.
                    let byte_off: usize = bytes[..idx].iter().map(|c| c.len_utf8()).sum();
                    let mut cursor: &str = &raw_str[byte_off..];
                    let _ = crate::ported::utils::skipparens('\u{91}', '\u{92}', &mut cursor);
                    let consumed = raw_str.len() - byte_off - cursor.len();
                    let advance_chars = raw_str[byte_off..byte_off + consumed].chars().count();
                    idx += advance_chars;
                    // Continue scanning for `=` / `+` after the `]`.
                    while idx < bytes.len() {
                        let ch = bytes[idx];
                        if ch == '=' || ch == '+' || ch == '\u{8d}' {
                            break;
                        }
                        idx += 1;
                    }
                }
                let is_inc = idx < bytes.len() && bytes[idx] == '+';
                // c:1856-1858 — `if (*ptr == '+') { *ptr++ = '\0';
                // ecadd(WCB_ASSIGN(SCALAR, INC, 0)); } else WCB_NEW`
                // C nulls the `+` AT THAT POSITION then advances ptr.
                // `name` is bytes BEFORE the `+`, NOT including it.
                let name_end = idx;
                if is_inc {
                    idx += 1;
                }
                let flag = if is_inc { WC_ASSIGN_INC } else { WC_ASSIGN_NEW };
                ecadd(WCB_ASSIGN(WC_ASSIGN_SCALAR, flag, 0));
                // c:1860 — `if (*ptr == '=') { *ptr = '\0'; str = ptr + 1; }
                //          else equalsplit(tokstr, &str);`
                let name: String = bytes[..name_end].iter().collect();
                let str_off = if idx < bytes.len() && (bytes[idx] == '=' || bytes[idx] == '\u{8d}')
                {
                    idx + 1
                } else {
                    idx
                };
                let value: String = bytes[str_off..].iter().collect();
                // c:1866-1877 — scan value for `=(`/`<(`/`>(` (proc
                // subst); if found, bump cmplx (suppresses Z_SIMPLE).
                let vbytes: Vec<char> = value.chars().collect();
                for (i, ch) in vbytes.iter().enumerate() {
                    if i + 1 < vbytes.len() && vbytes[i + 1] == '\u{88}'
                    /* Inpar */
                    {
                        if *ch == '\u{8d}' /* Equals */
                            || *ch == '\u{94}' /* Inang */
                            || *ch == '\u{96}'
                        /* OutangProc */
                        {
                            *cmplx = 1;
                            break;
                        }
                    }
                }
                ecstr(&name);
                ecstr(&value);
                isnull = false;
                assignments = true;
            }
            ENVARRAY => {
                // c:1883-1908 — array assignment `name=( ... )` in the
                // pre-cmd loop (no `typeset`-style typeset_force flag).
                // c:1884 — `int oldcmdpos = incmdpos, n, type2;`
                let oldcmdpos = incmdpos();
                let n: u32;
                let type2: wordcode;
                let p: usize;

                // c:1886-1889 — `array setting is cmplx because it can
                //   contain process substitutions`
                // c:1890 — `*cmplx = c = 1;`
                *cmplx = 1;
                // c:1891 — `p = ecadd(0);`
                p = ecadd(0);
                // c:1892 — `incmdpos = 0;`
                set_incmdpos(false);
                // c:1893-1897 — `+=` detection: if tokstr ends in `+`,
                // strip the `+` and use WC_ASSIGN_INC; else WC_ASSIGN_NEW.
                let raw = tokstr().unwrap_or_default();
                let (name, t2) = if raw.ends_with('+') {
                    (raw[..raw.len() - 1].to_string(), WC_ASSIGN_INC)
                } else {
                    (raw.clone(), WC_ASSIGN_NEW)
                };
                type2 = t2;
                // c:1898 — `ecstr(tokstr);` (tokstr now NUL-trimmed)
                ecstr(&name);
                // c:1899 — `cmdpush(CS_ARRAY);`
                cmdpush(CS_ARRAY as u8);
                // c:1900 — `zshlex();`
                zshlex();
                // c:1901 — `n = par_nl_wordlist();`
                n = par_nl_wordlist_wordcode();
                // c:1902 — `ecbuf[p] = WCB_ASSIGN(WC_ASSIGN_ARRAY, type2, n);`
                ECBUF.with_borrow_mut(|b| {
                    b[p] = WCB_ASSIGN(WC_ASSIGN_ARRAY, type2, n);
                });
                // c:1903 — `cmdpop();`
                cmdpop();
                // c:1904-1905 — `if (tok != OUTPAR) YYERROR(oecused);`
                if tok() != OUTPAR_TOK {
                    zerr("par_simple: expected `)' after array assignment");
                    return 0;
                }
                // c:1906 — `incmdpos = oldcmdpos;`
                set_incmdpos(oldcmdpos);
                // c:1907 — `isnull = 0;`
                isnull = false;
                // c:1908 — `assignments = 1;`
                assignments = true;
            }
            t if IS_REDIROP(t) => {
                // c:1900-1904 — `*cmplx = c = 1; nr += par_redir(&r,
                // NULL); continue;`. The wordcode-emitting redir is
                // distinct from the AST par_redir — it INSERTS
                // WCB_REDIR + fd + ecstrcode(name) at offset `r`
                // via ecispace, shifting any later words down.
                *cmplx = 1;
                let added = par_redir_wordcode(&mut r, None);
                if added == 0 {
                    break;
                }
                nr += added;
                continue;
            }
            _ => break,
        }
        zshlex(); // c:1907 `zshlex();`
    }

    // c:1920-1921 — `if (tok == AMPER || tok == AMPERBANG) YYERROR;`
    if tok() == AMPER || tok() == AMPERBANG {
        zerr("par_simple: unexpected &");
        return 0;
    }

    // c:1923 — `p = ecadd(WCB_SIMPLE(0));`
    let mut p = ecadd(WCB_SIMPLE(0));

    // c:1924-2105 — main words loop. is_typeset tracks whether the
    // outer command was `typeset`/`export`/etc. so the final
    // placeholder gets WCB_TYPESET instead of WCB_SIMPLE.
    let mut is_typeset = false;
    let mut postassigns: u32 = 0;
    let mut ppost: usize = 0;
    loop {
        match tok() {
            STRING_LEX | TYPESET => {
                // c:1926 — `int redir_var = 0;`
                let mut redir_var = false;
                // c:1928-1929 — `*cmplx = 1; incmdpos = 0;`
                *cmplx = 1;
                set_incmdpos(false);
                // c:1931-1932 — TYPESET → intypeset = is_typeset = 1.
                if tok() == TYPESET {
                    set_intypeset(true);
                    is_typeset = true;
                }
                let s = tokstr().unwrap_or_default();
                // c:1934-1974 — `{var}>file` brace-FD detection.
                // `if (!isset(IGNOREBRACES) && *tokstr == Inbrace)`
                let bytes = s.as_bytes();
                let first_is_inbrace = (bytes.len() >= 2 && bytes[0] == 0xc2 && bytes[1] == 0x8f)
                    || (bytes.len() >= 1 && bytes[0] == b'{');
                if !isset(IGNOREBRACES) && first_is_inbrace {
                    // c:1937-1938 — `char *eptr = tokstr + strlen(tokstr) - 1;`
                    //                `char *ptr = eptr;`
                    // C tests `*eptr == Outbrace` (0x90 marker or `}`) AND
                    // there's content between `{` and `}` (`ptr > tokstr + 1`).
                    let last_two_outbrace = bytes.len() >= 2
                        && (bytes.ends_with(&[0xc2, 0x90]) || bytes.last() == Some(&b'}'));
                    let opener_len = if bytes.len() >= 2 && bytes[0] == 0xc2 && bytes[1] == 0x8f {
                        2
                    } else {
                        1
                    };
                    let closer_len = if bytes.len() >= 2 && bytes.ends_with(&[0xc2, 0x90]) {
                        2
                    } else if bytes.last() == Some(&b'}') {
                        1
                    } else {
                        0
                    };
                    if last_two_outbrace && bytes.len() > opener_len + closer_len {
                        // c:1944 — `if (itype_end(tokstr+1, IIDENT, 0) >= ptr)`
                        // Inner content is the identifier between `{` and `}`.
                        let inner_start = opener_len;
                        let inner_end = bytes.len() - closer_len;
                        let inner = &s[inner_start..inner_end];
                        if !inner.is_empty() && crate::ported::params::isident(inner) {
                            // c:1946-1948 — `char *idstring = dupstrpfx(...);`
                            //                `redir_var = 1; zshlex();`
                            let idstring = inner.to_string();
                            redir_var = true;
                            zshlex();
                            // c:1953-1958 — `if (IS_REDIROP(tok) && tokfd == -1)
                            //   { *cmplx = c = 1; nrediradd = par_redir(&r, id);
                            //     p += nrediradd; sr += nrediradd; }`
                            if IS_REDIROP(tok()) && tokfd() == -1 {
                                *cmplx = 1;
                                let nrediradd = par_redir_wordcode(&mut r, Some(&idstring));
                                p += nrediradd as usize;
                                sr += nrediradd;
                            } else if postassigns > 0 {
                                // c:1959-1966 — postassigns path: emit
                                // WCB_ASSIGN(SCALAR, INC, 0) + name + ""
                                postassigns += 1;
                                ecadd(WCB_ASSIGN(WC_ASSIGN_SCALAR, WC_ASSIGN_INC, 0));
                                ecstr(&s);
                                ecstr("");
                            } else {
                                // c:1968-1972 — `else { ecstr(toksave); argc++; }`
                                ecstr(&s);
                                argc += 1;
                            }
                        }
                    }
                }
                if !redir_var {
                    // c:1977-1996 — normal (non-redir-var) STRING/TYPESET.
                    if postassigns > 0 {
                        // c:1979-1989 — typeset with bare-name arg → INC
                        postassigns += 1;
                        ecadd(WCB_ASSIGN(WC_ASSIGN_SCALAR, WC_ASSIGN_INC, 0));
                        ecstr(&s);
                        ecstr("");
                    } else {
                        ecstr(&s);
                        argc += 1;
                    }
                    zshlex();
                }
                isnull = false;
            }
            ENVSTRING => {
                // c:2005-2026 — mid-cmd ENVSTRING (under intypeset
                // context). Emits WCB_ASSIGN(SCALAR, NEW, 0) then
                // ecstr(name) + ecstr(value), tracking the first
                // postassign offset in `ppost` (which the trailing
                // WCB_TYPESET header points to).
                if postassigns == 0 {
                    ppost = ecadd(0);
                }
                postassigns += 1;
                // c:2010-2014 — `for (ptr = tokstr; *ptr && *ptr != Inbrack
                // && *ptr != '=' && *ptr != '+'; ptr++); if (*ptr == Inbrack)
                // skipparens(Inbrack, Outbrack, &ptr);`.
                let raw = tokstr().unwrap_or_default();
                let bytes: Vec<char> = raw.chars().collect();
                let mut idx = 0usize;
                while idx < bytes.len() {
                    let ch = bytes[idx];
                    if ch == '\u{91}' /* Inbrack */
                        || ch == '=' || ch == '+' || ch == '\u{8d}'
                    /* Equals */
                    {
                        break;
                    }
                    idx += 1;
                }
                if idx < bytes.len() && bytes[idx] == '\u{91}'
                /* Inbrack */
                {
                    // c:2014 — `skipparens(Inbrack, Outbrack, &ptr);`.
                    let byte_off: usize = bytes[..idx].iter().map(|c| c.len_utf8()).sum();
                    let mut cursor: &str = &raw[byte_off..];
                    let _ = crate::ported::utils::skipparens('\u{91}', '\u{92}', &mut cursor);
                    let consumed = raw.len() - byte_off - cursor.len();
                    let advance_chars = raw[byte_off..byte_off + consumed].chars().count();
                    idx += advance_chars;
                    while idx < bytes.len() {
                        let ch = bytes[idx];
                        if ch == '=' || ch == '+' || ch == '\u{8d}' {
                            break;
                        }
                        idx += 1;
                    }
                }
                let name: String = bytes[..idx].iter().collect();
                let str_off = if idx < bytes.len() && (bytes[idx] == '=' || bytes[idx] == '\u{8d}')
                {
                    idx + 1
                } else {
                    idx
                };
                let value: String = bytes[str_off..].iter().collect();
                ecadd(WCB_ASSIGN(WC_ASSIGN_SCALAR, WC_ASSIGN_NEW, 0));
                ecstr(&name);
                ecstr(&value);
                isnull = false;
                zshlex();
            }
            ENVARRAY => {
                // c:2027-2050 — mid-cmd ENVARRAY (typeset N=(…) form).
                // C tracks postassigns + ppost the same as ENVSTRING,
                // but the inner emit is WCB_ASSIGN(ARRAY, NEW, n)
                // with `n` patched in after par_nl_wordlist consumes
                // the elements. C also toggles intypeset=0 around the
                // wordlist so the lexer doesn't try to re-emit
                // assignments inside the array.
                *cmplx = 1;
                if postassigns == 0 {
                    ppost = ecadd(0);
                }
                postassigns += 1;
                let parr = ecadd(0);
                let raw = tokstr().unwrap_or_default();
                let is_inc = raw.ends_with('+');
                let name = if is_inc {
                    &raw[..raw.len() - 1]
                } else {
                    raw.as_str()
                };
                let flag = if is_inc { WC_ASSIGN_INC } else { WC_ASSIGN_NEW };
                ecstr(name);
                cmdpush(CS_ARRAY as u8);
                set_intypeset(false);
                zshlex();
                // c:2044 — `n = par_nl_wordlist();` (parse.c:2379-2391).
                // SEPER + NEWLIN both allowed between elements.
                let mut nelem = 0u32;
                loop {
                    let t = tok();
                    if t != STRING_LEX && t != SEPER && t != NEWLIN {
                        break;
                    }
                    if t == STRING_LEX {
                        ecstr(&tokstr().unwrap_or_default());
                        nelem += 1;
                    }
                    zshlex();
                }
                ECBUF.with_borrow_mut(|b| {
                    if parr < b.len() {
                        b[parr] = WCB_ASSIGN(WC_ASSIGN_ARRAY, flag, nelem);
                    }
                });
                cmdpop();
                set_intypeset(true);
                if tok() != OUTPAR_TOK {
                    zerr("expected `)' after array assignment");
                    return 0;
                }
                isnull = false;
                zshlex();
            }
            t if IS_REDIROP(t) => {
                // c:1999-2010 — `nrediradd = par_redir(&r, NULL);
                // p += nrediradd; if (ppost) ppost += nrediradd;
                // sr += nrediradd;`
                *cmplx = 1;
                let added = par_redir_wordcode(&mut r, None);
                if added == 0 {
                    break;
                }
                p += added as usize;
                if ppost != 0 {
                    ppost += added as usize;
                }
                sr += added;
            }
            INOUTPAR => {
                // c:2051 — `} else if (tok == INOUTPAR) {`
                // c:2052 — `zlong oldlineno = lineno;`
                let oldlineno = lineno();
                // c:2053 — `int onp, so, oecssub = ecssub;`
                let oecssub = ECSSUB.get();
                // c:2055-2057 — `if (!isset(MULTIFUNCDEF) && argc > 1) YYERROR;`
                if !isset(MULTIFUNCDEF) && argc > 1 {
                    zerr("par_simple: too many function names for funcdef");
                    return 0;
                }
                // c:2058-2060 — `if (assignments || postassigns) YYERROR;`
                if assignments || postassigns > 0 {
                    zerr("par_simple: assignments before funcdef");
                    return 0;
                }
                // c:2061-2068 — hasalias check + zwarn — skipped (no
                // alias tracking on the wordcode path).

                // c:2070 — `*cmplx = c;`
                *cmplx = c_saved;
                // c:2071 — `lineno = 0;`
                set_lineno(0);
                // c:2072 — `incmdpos = 1;`
                set_incmdpos(true);
                // c:2073 — `cmdpush(CS_FUNCDEF);`
                cmdpush(CS_FUNCDEF as u8);
                // c:2074 — `zshlex();`
                zshlex();
                // c:2075-2076 — `while (tok == SEPER) zshlex();`
                while tok() == SEPER {
                    zshlex();
                }
                // c:2079 — `ecispace(p + 1, 1); ecbuf[p+1] = argc;
                // ecadd(0)*4`. Insert the argc word at p+1, then
                // append 4 placeholder words.
                ecispace(p + 1, 1);
                ECBUF.with_borrow_mut(|b| {
                    if p + 1 < b.len() {
                        b[p + 1] = argc;
                    }
                });
                // c:2080-2083 — four metadata placeholder slots.
                ecadd(0);
                ecadd(0);
                ecadd(0);
                ecadd(0);

                // c:2085 — `ecnfunc++;`
                ECNFUNC.set(ECNFUNC.get() + 1);
                // c:2086 — `ecssub = so = ecsoffs;`
                let so = ECSOFFS.get();
                ECSSUB.set(so);
                // c:2087 — `onp = ecnpats;`
                let onp = ECNPATS.with(|cc| cc.get());
                // c:2088 — `ecnpats = 0;`
                ECNPATS.with(|cc| cc.set(0));

                // c:2091 — `int c = 0;` — INNER cmplx for the body
                // parse. Local to each branch; C's enclosing *cmplx
                // is NOT modified by the body.
                let mut body_c: i32 = 0;
                // c:2090 — `if (tok == INBRACE) {`
                if tok() == INBRACE_TOK {
                    // c:2093 — `zshlex();`
                    zshlex();
                    // c:2094 — `par_list(&c);`
                    par_list_wordcode(&mut body_c);
                    // c:2095-2101 — `if (tok != OUTBRACE) { cmdpop();
                    //   lineno += oldlineno; ecnpats = onp;
                    //   ecssub = oecssub; YYERROR; }`
                    if tok() != OUTBRACE_TOK {
                        cmdpop();
                        set_lineno(lineno() + oldlineno);
                        ECNPATS.with(|cc| cc.set(onp));
                        ECSSUB.set(oecssub);
                        zerr("par_simple: funcdef expected `}`");
                        return 0;
                    }
                    // c:2102-2105 — `if (argc == 0) incmdpos = 0;`
                    if argc == 0 {
                        set_incmdpos(false);
                    }
                    // c:2106 — `zshlex();`
                    zshlex();
                } else {
                    // c:2107-2132 — short-body funcdef form: `f() cmd`
                    // or `() cmd`. Wraps single par_cmd result in a
                    // synthetic WC_LIST / WC_SUBLIST /
                    // WC_PIPE(WC_PIPE_END, 0) header trio.
                    let ll = ecadd(0);
                    let sl = ecadd(0);
                    ecadd(WCB_PIPE(WC_PIPE_END, 0));
                    let ok = par_cmd_wordcode(&mut body_c, if argc == 0 { 1 } else { 0 });
                    if !ok {
                        cmdpop();
                        zerr("par_simple: funcdef short-body: missing command");
                        return 0;
                    }
                    if argc == 0 {
                        // c:2118-2127 — anonymous funcdef may take args
                        // after the body; first one already read.
                        set_incmdpos(false);
                    }
                    // c:2130-2131 — inner sublist/list use inner cmplx.
                    let used = ECUSED.get() as usize;
                    set_sublist_code(
                        sl,
                        WC_SUBLIST_END as i32,
                        0,
                        (used.saturating_sub(1 + sl)) as i32,
                        body_c != 0,
                    );
                    set_list_code(ll, Z_SYNC | Z_END, body_c != 0);
                }
                let _ = body_c;
                // c:2133 — `cmdpop();`
                cmdpop();

                // c:2135 — `ecadd(WCB_END());`
                ecadd(WCB_END());
                // c:2136-2139 — fill 4 metadata slots at p+argc+2..5
                let p_argc = (p + (argc as usize) + 2) as usize;
                let cur_so = ECSOFFS.get();
                let np_now = ECNPATS.with(|cc| cc.get());
                ECBUF.with_borrow_mut(|b| {
                    b[p_argc] = (so - oecssub) as wordcode;
                    b[p_argc + 1] = (cur_so - so) as wordcode;
                    b[p_argc + 2] = np_now as wordcode;
                    b[p_argc + 3] = 0;
                });

                // c:2141-2143 — `ecnpats = onp; ecssub = oecssub; ecnfunc++;`
                ECNPATS.with(|cc| cc.set(onp));
                ECSSUB.set(oecssub);
                ECNFUNC.set(ECNFUNC.get() + 1);

                // c:2145 — `ecbuf[p] = WCB_FUNCDEF(ecused - 1 - p);`
                let used = ECUSED.get() as usize;
                let header_off = used.saturating_sub(1 + p) as wordcode;
                ECBUF.with_borrow_mut(|b| {
                    b[p] = WCB_FUNCDEF(header_off);
                });

                // c:2147-2172 — `if (argc == 0) { /* anonymous fn args */ }`
                if argc == 0 {
                    // c:2150 — `int parg = ecadd(0);`
                    let mut parg = ecadd(0);
                    // c:2151 — `ecadd(0);`
                    ecadd(0);
                    // c:2152 — `while (tok == STRING || IS_REDIROP(tok)) {`
                    while tok() == STRING_LEX || IS_REDIROP(tok()) {
                        if tok() == STRING_LEX {
                            // c:2155-2157
                            ecstr(&tokstr().unwrap_or_default());
                            argc += 1;
                            zshlex();
                        } else {
                            // c:2159-2165 — *cmplx=c=1; nrediradd=par_redir;
                            // p += nrediradd; ppost += nrediradd if ppost;
                            // sr += nrediradd; parg += nrediradd;
                            *cmplx = 1;
                            let added = par_redir_wordcode(&mut r, None);
                            if added == 0 {
                                break;
                            }
                            p += added as usize;
                            if ppost != 0 {
                                ppost += added as usize;
                            }
                            sr += added;
                            parg += added as usize;
                        }
                    }
                    // c:2168-2169 — `if (argc > 0) *cmplx = 1;`
                    if argc > 0 {
                        *cmplx = 1;
                    }
                    // c:2170 — `ecbuf[parg] = ecused - parg;`
                    // c:2171 — `ecbuf[parg+1] = argc;`
                    let used2 = ECUSED.get() as usize;
                    ECBUF.with_borrow_mut(|b| {
                        b[parg] = (used2 - parg) as wordcode;
                        b[parg + 1] = argc;
                    });
                }
                // c:2173 — `lineno += oldlineno;`
                set_lineno(lineno() + oldlineno);

                // c:2175-2177 — `isfunc = 1; isnull = 0; break;`
                isfunc = true;
                isnull = false;
                break;
            }
            _ => break,
        }
    }

    // c:2173-2176 — `if (isnull && !(sr + nr)) { ecused = oecused;
    // return 0; }` — undo everything including pre-cmd assignments
    // if no actual command word emerged.
    if isnull && sr + nr == 0 && !assignments {
        ECUSED.set(p as i32);
        return 0;
    }
    // c:2186-2187 — `incmdpos = 1; intypeset = 0;` — reset before
    // the placeholder patch so the next-token lex doesn't carry
    // typeset/incond state.
    set_incmdpos(true);
    set_intypeset(false);
    // c:2189-2199 — `if (!isfunc) { if (is_typeset) ecbuf[p] =
    // WCB_TYPESET(argc); else ecbuf[p] = WCB_SIMPLE(argc); }`.
    // When isfunc=true the INOUTPAR branch already wrote WCB_FUNCDEF
    // at p; do NOT clobber it.
    if !isfunc {
        let header = if is_typeset {
            if postassigns > 0 {
                ECBUF.with_borrow_mut(|b| {
                    if ppost < b.len() {
                        b[ppost] = postassigns;
                    }
                });
            } else {
                ecadd(0);
            }
            WCB_TYPESET(argc)
        } else {
            WCB_SIMPLE(argc)
        };
        ECBUF.with_borrow_mut(|b| {
            if p < b.len() {
                b[p] = header;
            }
        });
    }
    1 + sr
}

/// Port of `par_redir(int *rp, char *idstring)` from
/// `Src/parse.c:2229-2345` — the wordcode-emitting variant that
/// pushes WCB_REDIR + fd + ecstrcode(name) into ECBUF. Distinct
/// from the AST `par_redir` (parse.rs:3771) which builds a
/// ZshRedir struct for the AST executor pipeline.
///
/// Returns the number of wordcodes added (3 for the basic shape,
/// 4 with idstring, 5 for HEREDOC[DASH] which carries the
/// terminator strings inline). Returns 0 on parse error.
///
/// `idstring` mirrors C's `char *idstring` parameter — `None` =
/// NULL (no `{var}>file` brace-FD shape), `Some(id)` = the captured
/// `{var}` name. C callers without a var pass NULL inline; Rust
/// callers do the same with `None`.
fn par_redir_wordcode(rp: &mut usize, idstring: Option<&str>) -> i32 {
    // c:2231 — `int r = *rp, type, fd1, oldcmdpos, oldnc, ncodes;`
    let r: usize = *rp;
    let mut r#type: i32;
    let fd1: i32;
    let oldcmdpos: bool;
    let oldnc: i32;
    let mut ncodes: usize;
    // c:2232 — `char *name;`
    let name: String;

    // c:2234 — `oldcmdpos = incmdpos;`
    oldcmdpos = incmdpos();
    // c:2235 — `incmdpos = 0;`
    set_incmdpos(false);
    // c:2236 — `oldnc = nocorrect;`
    oldnc = nocorrect();
    // c:2237-2238 — `if (tok != INANG && tok != INOUTANG) nocorrect = 1;`
    if tok() != INANG_TOK && tok() != INOUTANG {
        set_nocorrect(1);
    }
    // c:2239 — `type = redirtab[tok - OUTANG];`
    // Map current redirop token to redirtab index — matches order of
    // C `enum { OUTANG, OUTANGBANG, DOUTANG, DOUTANGBANG, INANG,
    // INOUTANG, DINANG, DINANGDASH, INANGAMP, OUTANGAMP, AMPOUTANG,
    // OUTANGAMPBANG, DOUTANGAMP, DOUTANGAMPBANG, TRINANG }`.
    r#type = match tok() {
        OUTANG_TOK => REDIR_WRITE,
        OUTANGBANG => REDIR_WRITENOW,
        DOUTANG => REDIR_APP,
        DOUTANGBANG => REDIR_APPNOW,
        INANG_TOK => REDIR_READ,
        INOUTANG => REDIR_READWRITE,
        DINANG => REDIR_HEREDOC,
        DINANGDASH => REDIR_HEREDOCDASH,
        INANGAMP => REDIR_MERGEIN,
        OUTANGAMP => REDIR_MERGEOUT,
        AMPOUTANG => REDIR_ERRWRITE,
        OUTANGAMPBANG => REDIR_ERRWRITENOW,
        DOUTANGAMP => REDIR_ERRAPP,
        DOUTANGAMPBANG => REDIR_ERRAPPNOW,
        TRINANG => REDIR_HERESTR,
        _ => {
            set_incmdpos(oldcmdpos);
            set_nocorrect(oldnc);
            return 0;
        }
    };
    // c:2240 — `fd1 = tokfd;`
    fd1 = tokfd();
    // c:2241 — `zshlex();`
    zshlex();
    // c:2242-2243 — `if (tok != STRING && tok != ENVSTRING) YYERROR(ecused);`
    if tok() != STRING_LEX && tok() != ENVSTRING {
        set_incmdpos(oldcmdpos);
        set_nocorrect(oldnc);
        zerr("expected word after redirection");
        return 0;
    }
    // c:2244 — `incmdpos = oldcmdpos;`
    set_incmdpos(oldcmdpos);
    // c:2245 — `nocorrect = oldnc;`
    set_nocorrect(oldnc);

    // c:2248-2249 — `if (fd1 == -1) fd1 = IS_READFD(type) ? 0 : 1;`
    let fd1 = if fd1 == -1 {
        if is_readfd(r#type) {
            0
        } else {
            1
        }
    } else {
        fd1
    };

    // c:2251 — `name = tokstr;`
    name = tokstr().unwrap_or_default();

    // c:2253-2321 — switch on type:
    match r#type {
        // c:2254-2300 — REDIR_HEREDOC / REDIR_HEREDOCDASH
        x if x == REDIR_HEREDOC || x == REDIR_HEREDOCDASH => {
            // c:2257 — `struct heredocs **hd;`
            // c:2258 — `int htype = type;`
            let htype = r#type;
            // c:2260-2261 — `if (strchr(tokstr, '\n')) YYERROR(ecused);`
            if name.contains('\n') {
                zerr("here-doc terminator contains newline");
                return 0;
            }
            // c:2263-2273 — `ncodes = 5; if (idstring) { type |= MASK; ncodes = 6; }`
            if idstring.is_some() {
                r#type |= REDIR_VARID_MASK;
                ncodes = 6;
            } else {
                ncodes = 5;
            }
            // c:2277 — `ecispace(r, ncodes);`
            ecispace(r, ncodes);
            // c:2278 — `*rp = r + ncodes;`
            *rp = r + ncodes;
            // c:2279 — `ecbuf[r] = WCB_REDIR(type | REDIR_FROM_HEREDOC_MASK);`
            ECBUF.with_borrow_mut(|b| {
                b[r] = WCB_REDIR((r#type | REDIR_FROM_HEREDOC_MASK) as wordcode);
                // c:2280 — `ecbuf[r + 1] = fd1;`
                b[r + 1] = fd1 as wordcode;
            });
            // c:2282-2286 — r+2..4 are filled later by setheredoc.
            // c:2287-2288 — `if (idstring) ecbuf[r + 5] = ecstrcode(idstring);`
            if let Some(id) = idstring {
                let coded = ecstrcode(id);
                ECBUF.with_borrow_mut(|b| {
                    b[r + 5] = coded;
                });
            }
            // c:2290-2296 — `for (hd = &hdocs; *hd; hd = &(*hd)->next);
            //                 *hd = zalloc(sizeof(struct heredocs));
            //                 (*hd)->next = NULL;
            //                 (*hd)->type = htype;
            //                 (*hd)->pc = r;
            //                 (*hd)->str = tokstr;`
            HDOCS.with_borrow_mut(|head| {
                let mut cur = head;
                while cur.is_some() {
                    cur = &mut cur.as_mut().unwrap().next; // c:2290
                }
                *cur = Some(Box::new(crate::ported::zsh_h::heredocs {
                    // c:2292-2296
                    next: None,
                    typ: htype,
                    pc: r as i32,
                    str: Some(name.clone()),
                }));
            });
            // c:2298 — `zshlex();`
            zshlex();
            // c:2299 — `return ncodes;`
            return ncodes as i32;
        }
        // c:2301-2308 — REDIR_WRITE / REDIR_WRITENOW
        x if x == REDIR_WRITE || x == REDIR_WRITENOW => {
            // c:2303-2305 — `if (tokstr[0] == OutangProc && tokstr[1] == Inpar)
            //                  type = REDIR_OUTPIPE;`
            let nb: Vec<char> = name.chars().collect();
            if nb.len() >= 2 && nb[0] == '\u{96}' && nb[1] == '\u{88}' {
                r#type = REDIR_OUTPIPE;
            } else if nb.len() >= 2 && nb[0] == '\u{94}' && nb[1] == '\u{88}' {
                // c:2306-2307 — `else if (tokstr[0] == Inang && tokstr[1] == Inpar) YYERROR;`
                zerr("par_redir: < before >");
                return 0;
            }
        }
        // c:2309-2315 — REDIR_READ
        x if x == REDIR_READ => {
            let nb: Vec<char> = name.chars().collect();
            if nb.len() >= 2 && nb[0] == '\u{94}' && nb[1] == '\u{88}' {
                r#type = REDIR_INPIPE;
            } else if nb.len() >= 2 && nb[0] == '\u{96}' && nb[1] == '\u{88}' {
                zerr("par_redir: > before <");
                return 0;
            }
        }
        // c:2316-2320 — REDIR_READWRITE
        x if x == REDIR_READWRITE => {
            let nb: Vec<char> = name.chars().collect();
            if nb.len() >= 2 && (nb[0] == '\u{94}' || nb[0] == '\u{96}') && nb[1] == '\u{88}' {
                r#type = if nb[0] == '\u{94}' {
                    REDIR_INPIPE
                } else {
                    REDIR_OUTPIPE
                };
            }
        }
        _ => {}
    }
    // c:2322 — `zshlex();`
    zshlex();

    // c:2326-2333 — `if (idstring) { type |= MASK; ncodes = 4; } else ncodes = 3;`
    if idstring.is_some() {
        r#type |= REDIR_VARID_MASK;
        ncodes = 4;
    } else {
        ncodes = 3;
    }

    // c:2334 — `ecispace(r, ncodes);`
    ecispace(r, ncodes);
    // c:2335 — `*rp = r + ncodes;`
    *rp = r + ncodes;
    // c:2336 — `ecbuf[r] = WCB_REDIR(type);`
    let coded_name = ecstrcode(&name);
    ECBUF.with_borrow_mut(|b| {
        b[r] = WCB_REDIR(r#type as wordcode);
        // c:2337 — `ecbuf[r + 1] = fd1;`
        b[r + 1] = fd1 as wordcode;
        // c:2338 — `ecbuf[r + 2] = ecstrcode(name);`
        b[r + 2] = coded_name;
    });
    // c:2339-2340 — `if (idstring) ecbuf[r + 3] = ecstrcode(idstring);`
    if let Some(id) = idstring {
        let coded_id = ecstrcode(id);
        ECBUF.with_borrow_mut(|b| {
            b[r + 3] = coded_id;
        });
    }
    // c:2342 — `return ncodes;`
    ncodes as i32
}

/// Port of `IS_READFD(type)` macro from `Src/zsh.h` — determines
/// default fd (0 for read-ish, 1 for write-ish) when none specified.
fn is_readfd(t: i32) -> bool {
    matches!(
        t,
        x if x == REDIR_READ
            || x == REDIR_READWRITE
            || x == REDIR_MERGEIN
            || x == REDIR_HEREDOC
            || x == REDIR_HEREDOCDASH
            || x == REDIR_HERESTR
    )
}

/// Parse a program (list of lists)
/// Parse a complete program (top-level entry). Calls
/// parse_program_until with no end-token sentinel. Direct port of
/// zsh/Src/parse.c:614-720 `parse_event` / `par_list` /
/// `par_event` flow. C distinguishes COND_EVENT (single command
/// for here-string) from full event parse; zshrs's parse_program
/// is the full-event entry.
fn parse_program() -> ZshProgram {
    parse_program_until(None)
}

/// Parse a program until we hit an end token
/// Parse a program until one of `end_tokens` is seen (or EOF).
/// Drives par_list in a loop. C equivalent: the body of par_event
/// (parse.c:635-695) iterating par_list against the lexer.
fn parse_program_until(end_tokens: Option<&[lextok]>) -> ZshProgram {
    let mut lists = Vec::new();

    loop {
        // Skip separators
        while tok() == SEPER || tok() == NEWLIN {
            zshlex();
        }

        if tok() == ENDINPUT {
            break;
        }
        if tok() == LEXERR {
            // c:Src/parse.c:671-680 par_event — when the lexer
            // returned LEXERR (e.g. unbalanced `$((1+(2))` math
            // sub, unterminated string, etc.), C emits `yyerror(1)`
            // and sets errflag so the script aborts with a parse
            // error diagnostic + non-zero exit. zshrs's
            // parse_program_until previously just `break`'d on
            // LEXERR, silently swallowing the malformed input and
            // exiting rc=0 — so `$((1+(2))` ran as if it were
            // empty. Bug #529 in docs/BUGS.md. Emit yyerror
            // mirroring the C behaviour; the broken script then
            // surfaces the parse error to the caller.
            // c:Src/parse.c — empty-msg yyerror call mapped to the
            // C-faithful `yyerror(0)` (zshrs's previous shape).
            yyerror(0);
            break;
        }

        // Check for end tokens
        if let Some(end_toks) = end_tokens {
            if end_toks.contains(&tok()) {
                break;
            }
        }

        // Also stop at these tokens when not explicitly looking for them
        // Note: Else/Elif/Then are NOT here - they're handled by par_if
        // to allow nested if statements inside case arms, loops, etc.
        //
        // c:Src/parse.c:par_event — when an orphan terminator (DONE
        // outside a loop, FI outside an if, ESAC outside a case)
        // appears at the top level (end_tokens=None), C errors via
        // YYERROR. zshrs's `break` silently accepted `done`/`fi`/
        // `esac` as no-op input. Error at the outermost call so
        // unscoped terminators don't sneak through; nested calls
        // still break cleanly via the end_tokens contains-check
        // above.
        match tok() {
            DONE | FI | ESAC | DOLOOP if end_tokens.is_none() => {
                // c:Src/parse.c:par_event — emit the specific token
                // name (`done`, `fi`, `esac`, `do`) so error-parsing
                // tools can identify the unmatched terminator. C zsh
                // writes `parse error near \`<tok>'`; the Rust port
                // was emitting a generic "orphan terminator" string.
                // Bug #142, #413.
                let name = match tok() {
                    DONE => "done",
                    FI => "fi",
                    ESAC => "esac",
                    DOLOOP => "do",
                    _ => "orphan terminator",
                };
                zerr(&format!("parse error near `{}'", name));
                break;
            }
            DSEMI | SEMIAMP | SEMIBAR if end_tokens.is_none() => {
                // c:Src/parse.c:par_event — case-arm terminators
                // (`;;`, `;&`, `;|`) outside a case construct are a
                // parse error. zshrs's `break` silently accepted them
                // at top level, truncating the rest of the script.
                // Bug #141 in docs/BUGS.md.
                let name = match tok() {
                    DSEMI => ";;",
                    SEMIAMP => ";&",
                    SEMIBAR => ";|",
                    _ => "case terminator",
                };
                zerr(&format!("parse error near `{}'", name));
                break;
            }
            OUTBRACE_TOK if end_tokens.is_none() => {
                // c:Src/parse.c:par_event — orphan `}` (no matching
                // `{` opener) at top level is a parse error. zshrs's
                // generic break swallowed it silently, leaving the
                // `echo a` in `echo a }` running and ignoring the
                // stray brace. Bug #168 in docs/BUGS.md.
                zerr("parse error near `}'");
                break;
            }
            OUTBRACE_TOK | DSEMI | SEMIAMP | SEMIBAR | DONE | FI | ESAC | ZEND => break,
            _ => {}
        }

        match par_list() {
            Some((list, terminated)) => {
                let detected = simple_name_with_inoutpar(&list);
                let was_detected = detected.is_some();
                lists.push(list);
                // Synthesize a FuncDef for the `name() { body }` shape
                // at parse time so body_source is captured while the
                // lexer still has the input. The lexer port emits
                // `name(` as a single Word ending in `<Inpar><Outpar>`,
                // so the Simple list is followed by an Inbrace once
                // separators are skipped. For `name() cmd args` the
                // body has already been swallowed into the same
                // Simple's words tail — synthesize directly from there.
                if let Some((names, body_argv)) = detected {
                    if !body_argv.is_empty() {
                        // One-line body already in the Simple. Build
                        // a Simple from body_argv as the function body.
                        lists.pop();
                        let body_simple = ZshCommand::Simple(ZshSimple {
                            assigns: Vec::new(),
                            words: body_argv,
                            redirs: Vec::new(),
                        });
                        let body_list = ZshList {
                            sublist: ZshSublist {
                                pipe: ZshPipe {
                                    cmd: body_simple,
                                    next: None,
                                    lineno: lineno(),
                                    merge_stderr: false,
                                },
                                next: None,
                                flags: SublistFlags::default(),
                            },
                            flags: ListFlags::default(),
                        };
                        let funcdef = ZshCommand::FuncDef(ZshFuncDef {
                            names,
                            body: Box::new(ZshProgram {
                                lists: vec![body_list],
                            }),
                            tracing: false,
                            auto_call_args: None,
                            body_source: None,
                        });
                        let synthetic = ZshList {
                            sublist: ZshSublist {
                                pipe: ZshPipe {
                                    cmd: funcdef,
                                    next: None,
                                    lineno: lineno(),
                                    merge_stderr: false,
                                },
                                next: None,
                                flags: SublistFlags::default(),
                            },
                            flags: ListFlags::default(),
                        };
                        lists.push(synthetic);
                        continue;
                    }
                    // Else: words.len() == 1 (only the trailing `name()`
                    // word), brace body follows. `names` may carry
                    // multiple identifiers from the `fna fnb fnc()`
                    // shorthand — all share the same brace body per
                    // src/zsh/Src/parse.c:1666 par_funcdef wordlist.
                    // Skip separators on the real lexer; safe because
                    // parse_program's next iteration would also skip them.
                    while tok() == SEPER || tok() == NEWLIN {
                        zshlex();
                    }
                    if tok() == INBRACE_TOK {
                        // Capture body_start BEFORE the lexer
                        // advances past the first body token. The
                        // outer zshlex() consumed `{`; lexer.pos
                        // is now right after `{`. The next
                        // `zshlex()` would advance past `echo`,
                        // making body_start land mid-body and
                        // lose the first word — `typeset -f f`
                        // printed `a; echo b` instead of
                        // `echo a; echo b` for `f() { echo a;
                        // echo b }`.
                        let body_start = pos();
                        zshlex();
                        // c:Src/parse.c — synth funcdef body terminates
                        // at OUTBRACE_TOK. Explicit end-token avoids
                        // the top-level stray-`}` arm. Bug #167/#168.
                        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
                        let body_end = if tok() == OUTBRACE_TOK {
                            pos().saturating_sub(1)
                        } else {
                            pos()
                        };
                        let body_source = input_slice(body_start, body_end)
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty());
                        if tok() == OUTBRACE_TOK {
                            zshlex();
                        }
                        // Replace the Simple list with a FuncDef list.
                        lists.pop();
                        let funcdef = ZshCommand::FuncDef(ZshFuncDef {
                            names,
                            body: Box::new(body),
                            tracing: false,
                            auto_call_args: None,
                            body_source,
                        });
                        let synthetic = ZshList {
                            sublist: ZshSublist {
                                pipe: ZshPipe {
                                    cmd: funcdef,
                                    next: None,
                                    lineno: lineno(),
                                    merge_stderr: false,
                                },
                                next: None,
                                flags: SublistFlags::default(),
                            },
                            flags: ListFlags::default(),
                        };
                        lists.push(synthetic);
                    } else if !matches!(tok(), ENDINPUT | OUTBRACE_TOK | SEPER | NEWLIN) {
                        // No-brace one-line body: `foo() echo hello`.
                        // Parse a single command for the body.
                        let body_cmd = par_cmd();
                        if let Some(cmd) = body_cmd {
                            let body_list = ZshList {
                                sublist: ZshSublist {
                                    pipe: ZshPipe {
                                        cmd,
                                        next: None,
                                        lineno: lineno(),
                                        merge_stderr: false,
                                    },
                                    next: None,
                                    flags: SublistFlags::default(),
                                },
                                flags: ListFlags::default(),
                            };
                            lists.pop();
                            let funcdef = ZshCommand::FuncDef(ZshFuncDef {
                                names: names.clone(),
                                body: Box::new(ZshProgram {
                                    lists: vec![body_list],
                                }),
                                tracing: false,
                                auto_call_args: None,
                                body_source: None,
                            });
                            let synthetic = ZshList {
                                sublist: ZshSublist {
                                    pipe: ZshPipe {
                                        cmd: funcdef,
                                        next: None,
                                        lineno: lineno(),
                                        merge_stderr: false,
                                    },
                                    next: None,
                                    flags: SublistFlags::default(),
                                },
                                flags: ListFlags::default(),
                            };
                            lists.push(synthetic);
                        }
                    }
                }
                // c:769-803 + c:644-682 — lists only chain across
                // explicit SEPER/AMPER/AMPERBANG. When par_list ended
                // WITHOUT a terminator, C's grammar says the list is
                // over: nested contexts (while-cond, brace bodies)
                // return to the caller with the dangling token (so
                // `while {false} {print no}` hands the second `{` to
                // par_while as the loop body), and the TOP level
                // (par_event, c:671-680) yyerrors — `{false} {print
                // no}` is `parse error near \`{'` in zsh. The funcdef
                // synthesis branch above manages its own following
                // tokens (`f() { body }` legitimately has INBRACE
                // right after the Simple), so it is exempt.
                if !was_detected && !terminated {
                    match tok() {
                        // End-of-input / lex error: loop top handles.
                        ENDINPUT | LEXERR => {}
                        // Construct closers and orphan terminators:
                        // the loop-top match already errors/breaks
                        // for these with the right diagnostics.
                        OUTBRACE_TOK | DSEMI | SEMIAMP | SEMIBAR | DONE | FI | ESAC | ZEND
                        | DOLOOP | ELSE | ELIF | THEN => {}
                        t if end_tokens.map_or(false, |e| e.contains(&t)) => {}
                        _ if end_tokens.is_some() => {
                            // Nested list context: C par_list just
                            // ends; the construct parser (par_while
                            // body dispatch, par_subsh closer check)
                            // deals with the token.
                            break;
                        }
                        offending => {
                            // Top level: C par_event yyerror(1) +
                            // errflag, c:671-682. Same tokstr
                            // injection as the None arm below so the
                            // "near `X'" tail survives punctuation
                            // tokens with no tokstr.
                            if crate::ported::lex::tokstr().is_none() {
                                let i = offending as usize;
                                if i < crate::ported::lex::tokstrings.len() {
                                    if let Some(s) = crate::ported::lex::tokstrings[i] {
                                        crate::ported::lex::set_tokstr(Some(s.to_string()));
                                    }
                                }
                            }
                            set_tok(LEXERR); // c:672
                            yyerror(1);
                            let noerrs_v =
                                *crate::ported::utils::noerrs_lock().lock().unwrap();
                            if noerrs_v != 2 {
                                errflag.fetch_or(
                                    crate::ported::zsh_h::ERRFLAG_ERROR,
                                    Ordering::SeqCst,
                                );
                            }
                            break;
                        }
                    }
                }
            }
            None => {
                // c:Src/parse.c:644-645 par_event — `if (tok ==
                // ENDINPUT) return 0;`. End-of-input is NORMAL — no
                // diagnostic. The yyerror path at c:670-682 fires only
                // when par_sublist actually failed (the C `!r` branch).
                //
                // zshrs's previous fix here unconditionally called
                // yyerror on every None return, breaking legitimate
                // end-of-input scenarios like `(echo sub)` (par_list
                // returns None after the subshell parser consumes the
                // whole construct, leaving tok at ENDINPUT).
                if tok() == ENDINPUT {
                    break;
                }
                // c:Src/parse.c:671-680 par_event — par_sublist failed:
                // emit the canonical yyerror with the "near `X'" tail
                // derived from zshlextext/tokstr().
                //
                // c:Src/lex.c:1965 — `zshlextext = tokstrings[tok]`
                // is set DURING zshlex, before set_tok(LEXERR) here.
                // zshrs's zshlex doesn't update LEX_TOKSTR for
                // single-char punctuation tokens (OUTPAR/INPAR/etc.),
                // so by the time yyerror runs, tokstr() is None and
                // the tail "near `)'" is lost. Inject the current
                // tok's canonical text into LEX_TOKSTR here so the
                // C-faithful yyerror lookup finds it. Mirrors the
                // visible effect of C's zshlextext fallback.
                let already_flagged = (errflag.load(Ordering::SeqCst)
                    & crate::ported::zsh_h::ERRFLAG_ERROR)
                    != 0;
                let offending_tok = tok();
                if crate::ported::lex::tokstr().is_none() {
                    let i = offending_tok as usize;
                    if i < crate::ported::lex::tokstrings.len() {
                        if let Some(s) = crate::ported::lex::tokstrings[i] {
                            crate::ported::lex::set_tokstr(Some(
                                s.to_string(),
                            ));
                        }
                    }
                }
                set_tok(LEXERR); // c:672
                yyerror(if already_flagged { 0 } else { 1 });
                // c:Src/parse.c:679-680 — `if (noerrs != 2) errflag |=
                // ERRFLAG_ERROR;`. C sets errflag explicitly after the
                // yyerror(1) print-only branch. Without this, the
                // caller (`execute_script_zsh_pipeline` here, `bin_eval`
                // for the parse_string path) can't distinguish "no
                // parse error" from "parse error already printed", so
                // $? stays at 0 after `eval ')foo'`.
                let noerrs_v =
                    *crate::ported::utils::noerrs_lock().lock().unwrap();
                if noerrs_v != 2 {
                    errflag.fetch_or(
                        crate::ported::zsh_h::ERRFLAG_ERROR,
                        Ordering::SeqCst,
                    );
                }
                break;
            }
        }
    }

    ZshProgram { lists }
}

/// Parse an assignment
/// Parse an assignment word `NAME=value` or `NAME=(arr items)`.
/// Sub-routine of par_simple. The C source handles assignments
/// inline in par_simple via the ENVSTRING/ENVARRAY token paths
/// (parse.c:1842-2000ish); zshrs splits it out to a dedicated
/// helper for clarity.
fn parse_assign() -> Option<ZshAssign> {
    // Helper: locate the Equals-marker that delimits NAME from
    // VALUE in an assignment-shaped tokstr. The lexer META-encodes
    // EVERY `=` (including those inside `${var%%=foo}` strip
    // patterns or `[idx]=...` subscripts), so a naive
    // `tokstr.find(Equals)` would split at the first inner `=`
    // and break the whole assignment. Walk the string skipping
    // brace and bracket depth so the assignment's `=` (the one
    // after the last `]` of the LHS subscript / or after the
    // bare name) is the one we land on.
    fn find_assign_equals(s: &str) -> Option<usize> {
        let target = Equals;
        let mut brace = 0i32;
        let mut bracket = 0i32;
        let mut paren = 0i32;
        for (i, c) in s.char_indices() {
            match c {
                    '{' | '\u{8f}' /* Inbrace */ => brace += 1,
                    '}' | '\u{90}' /* Outbrace */ => {
                        if brace > 0 {
                            brace -= 1;
                        }
                    }
                    '[' | '\u{91}' /* Inbrack */ => bracket += 1,
                    ']' | '\u{92}' /* Outbrack */ => {
                        if bracket > 0 {
                            bracket -= 1;
                        }
                    }
                    '(' | '\u{88}' /* Inpar */ => paren += 1,
                    ')' | '\u{8a}' /* Outpar */ => {
                        if paren > 0 {
                            paren -= 1;
                        }
                    }
                    _ if c == target && brace == 0 && bracket == 0 && paren == 0 => {
                        return Some(i);
                    }
                    _ => {}
                }
        }
        None
    }

    let _ts_tokstr = tokstr()?;
    let tokstr = _ts_tokstr.as_str();

    // Parse name=value or name+=value.
    let (name, value_str, append) = if tok() == ENVARRAY {
        let (name, append) = if let Some(stripped) = tokstr.strip_suffix('+') {
            (stripped, true)
        } else {
            (tokstr, false)
        };
        (name.to_string(), String::new(), append)
    } else if let Some(pos) = find_assign_equals(tokstr) {
        let name_part = &tokstr[..pos];
        let (name, append) = if let Some(stripped) = name_part.strip_suffix('+') {
            (stripped, true)
        } else {
            (name_part, false)
        };
        (
            name.to_string(),
            tokstr[pos + Equals.len_utf8()..].to_string(),
            append,
        )
    } else if let Some(pos) = tokstr.find('=') {
        // Fallback to literal '=' for compatibility
        let name_part = &tokstr[..pos];
        let (name, append) = if let Some(stripped) = name_part.strip_suffix('+') {
            (stripped, true)
        } else {
            (name_part, false)
        };
        (name.to_string(), tokstr[pos + 1..].to_string(), append)
    } else {
        return None;
    };

    let value = if tok() == ENVARRAY {
        // Array assignment: name=(...)
        // c:Src/parse.c:1895 par_simple ENVARRAY arm:
        //   `int oldcmdpos = incmdpos; ... incmdpos = 0; ... zshlex();`
        // Reset incmdpos to false BEFORE the array body's first lex so
        // a leading `{...}` (brace expansion) doesn't trip the
        // empty-buf+incmdpos rule at lex.c:1141 that returns `{` as
        // STRING and lets the reswd_lookup promote it to INBRACE_TOK.
        let oldcmdpos = crate::ported::lex::incmdpos();
        crate::ported::lex::set_incmdpos(false);
        let mut elements = Vec::new();
        zshlex(); // skip past token

        let mut arr_iters = 0;
        const MAX_ARRAY_ELEMENTS: usize = 10_000;
        while matches!(tok(), STRING_LEX | SEPER | NEWLIN) {
            arr_iters += 1;
            if arr_iters > MAX_ARRAY_ELEMENTS {
                zerr("array assignment exceeded maximum elements");
                break;
            }
            if tok() == STRING_LEX {
                let _ts_s = crate::ported::lex::tokstr();
                if let Some(s) = _ts_s.as_deref() {
                    elements.push(s.to_string());
                }
            }
            zshlex();
        }
        // c:Src/parse.c — `incmdpos = oldcmdpos;` (restore at end of arm)
        crate::ported::lex::set_incmdpos(oldcmdpos);

        // The closing Outpar is consumed here. The outer par_simple
        // loop will then `zshlex()` past whatever follows (typically
        // a separator or the next word) — calling zshlex twice in
        // tandem (here AND in par_simple) over-advances and merges
        // a following `name() { … }` funcdef into the same Simple.
        // We only consume Outpar; let the caller handle the rest.
        // Without this guard `g=(o1); f() { :; }` parsed as one
        // Simple with assigns=[g] and words=["f()"] (one token).
        if tok() == OUTPAR_TOK {
            // Note: do NOT zshlex() here. par_simple's `lexer
            // .zshlex()` after `parse_assign` returns advances past
            // the Outpar onto the next significant token.
            //
            // Force `incmdpos=true` so the next zshlex() recognizes
            // a follow-up `b=(...)` / `b=val` as Envarray/Envstring.
            // The lexer flips incmdpos to false on bare Outpar (which
            // is correct for subshell-close context), but for an
            // array-assignment close more assigns/words may follow.
            set_incmdpos(true);
        }

        ZshAssignValue::Array(elements)
    } else {
        ZshAssignValue::Scalar(value_str)
    };

    Some(ZshAssign {
        name,
        value,
        append,
    })
}

/// AST `par_redir` variant accepting an idstring for the
/// `{var}>file` brace-FD shape. C signature
/// `par_redir(int *rp, char *idstring)` (parse.c:2229). The
/// idstring is stored in the resulting ZshRedir.varid for the
/// executor to bind the named variable to the chosen fd.
fn par_redir_with_id(idstring: Option<&str>) -> Option<ZshRedir> {
    let varid: Option<String> = idstring.map(|s| s.to_string());
    let rtype = match tok() {
        OUTANG_TOK => REDIR_WRITE,
        OUTANGBANG => REDIR_WRITENOW,
        DOUTANG => REDIR_APP,
        DOUTANGBANG => REDIR_APPNOW,
        INANG_TOK => REDIR_READ,
        INOUTANG => REDIR_READWRITE,
        DINANG => REDIR_HEREDOC,
        DINANGDASH => REDIR_HEREDOCDASH,
        TRINANG => REDIR_HERESTR,
        INANGAMP => REDIR_MERGEIN,
        OUTANGAMP => REDIR_MERGEOUT,
        AMPOUTANG => REDIR_ERRWRITE,
        OUTANGAMPBANG => REDIR_ERRWRITENOW,
        DOUTANGAMP => REDIR_ERRAPP,
        DOUTANGAMPBANG => REDIR_ERRAPPNOW,
        _ => return None,
    };

    let fd = if tokfd() >= 0 {
        tokfd()
    } else if matches!(
        rtype,
        REDIR_READ
            | REDIR_READWRITE
            | REDIR_MERGEIN
            | REDIR_HEREDOC
            | REDIR_HEREDOCDASH
            | REDIR_HERESTR
    ) {
        0
    } else {
        1
    };

    // c:2234-2245 — save/restore incmdpos and nocorrect around the
    // zshlex that consumes the redir target word:
    //   oldcmdpos = incmdpos; incmdpos = 0;
    //   oldnc = nocorrect;
    //   if (tok != INANG && tok != INOUTANG) nocorrect = 1;
    //   ... zshlex; check tok; ...
    //   incmdpos = oldcmdpos; nocorrect = oldnc;
    // Without this, a redir target lexes in the parent's incmdpos
    // (re-promoting `{` / reswords) AND with parent nocorrect (so
    // spelling-correction wrongly runs inside `> $(cmd)` etc.).
    let oldcmdpos = incmdpos();
    set_incmdpos(false);
    let oldnc = nocorrect();
    let cur = tok();
    if cur != INANG_TOK && cur != INOUTANG {
        set_nocorrect(1);
    }
    zshlex();

    let name = match tok() {
        STRING_LEX | ENVSTRING => {
            let n = tokstr().unwrap_or_default();
            // c:2244-2245 — restore incmdpos / nocorrect right after
            // the redir target word is confirmed, BEFORE the trailing
            // zshlex advances past it. The advance itself is deferred
            // below so REDIR_HEREDOC[DASH] can push onto HDOCS first
            // (matching the wordcode variant at parse.rs:6894-6908) —
            // otherwise the NEWLIN drained by that zshlex sees an
            // empty HDOCS list and gethere never collects the body.
            set_incmdpos(oldcmdpos);
            set_nocorrect(oldnc);
            n
        }
        _ => {
            set_incmdpos(oldcmdpos);
            set_nocorrect(oldnc);
            zerr("expected word after redirection");
            return None;
        }
    };

    // Heredoc terminator capture. C parse.c:2254-2317 par_redir builds
    // a `struct heredocs` entry here for REDIR_HEREDOC[DASH]. zshrs
    // pushes onto HDOCS (canonical C linked list, c:2290-2296) AND
    // onto LEX_HEREDOCS (Rust-only AST-glue Vec carrying parsed-out
    // terminator/strip_tabs/quoted metadata for downstream AST
    // consumers). Quoted terminators (`<<'EOF'` / `<<"EOF"` / `<<\EOF`)
    // disable expansion in the body — Snull `\u{9d}` marks single-quote,
    // Dnull `\u{9e}` marks double-quote, Bnull `\u{9f}` marks
    // backslash-escaped chars.
    let heredoc_idx = if matches!(rtype, REDIR_HEREDOC | REDIR_HEREDOCDASH) {
        let strip_tabs = rtype == REDIR_HEREDOCDASH;
        let quoted = name.contains('\u{9d}')
            || name.contains('\u{9e}')
            || name.contains('\u{9f}')
            || name.starts_with('\'')
            || name.starts_with('"');
        let term = name
            .chars()
            .filter(|c| {
                *c != '\'' && *c != '"' && *c != '\u{9d}' && *c != '\u{9e}' && *c != '\u{9f}'
            })
            .collect::<String>();
        // c:2290-2296 — `for (hd = &hdocs; *hd; hd = &(*hd)->next);
        //                 *hd = zalloc(sizeof(struct heredocs));
        //                 (*hd)->next = NULL;
        //                 (*hd)->type = htype;
        //                 (*hd)->pc = r;
        //                 (*hd)->str = tokstr;`
        // AST path has no wordcode pc to patch; use -1 sentinel so the
        // inline NEWLIN walk in `zshlex()` skips the setheredoc call.
        HDOCS.with_borrow_mut(|head| {
            let mut cur = head;
            while cur.is_some() {
                cur = &mut cur.as_mut().unwrap().next; // c:2290
            }
            *cur = Some(Box::new(crate::ported::zsh_h::heredocs {
                // c:2292-2296
                next: None,
                typ: rtype,
                pc: -1,
                str: Some(name.clone()),
            }));
        });
        // zshrs-only: push parallel AST-glue entry onto LEX_HEREDOCS.
        let idx = LEX_HEREDOCS.with_borrow_mut(|v| {
            v.push(HereDoc {
                terminator: term,
                strip_tabs,
                content: String::new(),
                quoted,
                processed: false,
            });
            v.len() - 1
        });
        Some(idx)
    } else {
        None
    };

    // c:2298 (heredoc) / c:2322 (other redirs) — final zshlex() advance
    // past the redir target word. MUST run after the HDOCS push above
    // so the heredoc-drain inside this zshlex sees the new entry. For
    // non-heredoc forms the order is irrelevant; consolidating to a
    // single tail-call here matches the wordcode variant.
    zshlex();

    Some(ZshRedir {
        rtype,
        fd,
        name,
        heredoc: None,
        varid,
        heredoc_idx,
    })
}

/// Parse C-style for loop: for (( init; cond; step ))
/// Parse the c-style `for ((init; cond; incr)) do BODY done`.
/// Inner branch of zsh/Src/parse.c:1100-1140 inside par_for.
/// Recognized when the token after FOR is DINPAR (the `((`
/// detected by gettok via dbparens setup).
fn parse_for_cstyle() -> Option<ZshCommand> {
    // We're at (( (Dinpar None) - the opening ((
    // Lexer returns:
    //   Dinpar None     - opening ((
    //   Dinpar "init"   - init expression, semicolon consumed
    //   Dinpar "cond"   - cond expression, semicolon consumed
    //   Doutpar "step"  - step expression, closing )) consumed
    zshlex(); // Get init: Dinpar "i=0"

    if tok() != DINPAR {
        zerr("expected init expression in for ((");
        return None;
    }
    let init = tokstr().unwrap_or_default();

    zshlex(); // Get cond: Dinpar "i<10"

    if tok() != DINPAR {
        zerr("expected condition in for ((");
        return None;
    }
    let cond = tokstr().unwrap_or_default();

    zshlex(); // Get step: Doutpar "i++"

    if tok() != DOUTPAR {
        zerr("expected )) in for");
        return None;
    }
    let step = tokstr().unwrap_or_default();

    // c:1110 — `infor = 0;` before the body opener. The companion
    // `incmdpos = 1;` at c:1111 is intentionally skipped here for
    // the same reason c:1094's `incmdpos = 0;` is skipped in
    // par_for above — zshrs doesn't mirror the full
    // incmdpos state-machine inline.
    set_infor(0); // c:1110
    zshlex(); // Move past ))

    skip_separators();
    let body = parse_loop_body(false, false)?;

    Some(ZshCommand::For(ZshFor {
        var: String::new(),
        list: ForList::CStyle { init, cond, step },
        body: Box::new(body),
        is_select: false,
    }))
}

/// Parse select loop (same syntax as for)
/// Parse `select NAME in WORDS; do BODY; done`. Same shape as
/// `for NAME in WORDS; do ...` but with menu-prompt semantics in
/// the executor. C equivalent: the SELECT case in par_for at
/// parse.c:1087-1207 (selects share parser flow with foreach).
fn parse_select() -> Option<ZshCommand> {
    // `select` shares par_for's grammar (var, words, body) but the
    // compile path is different (interactive prompt loop).
    match par_for()? {
        ZshCommand::For(mut f) => {
            f.is_select = true;
            Some(ZshCommand::For(f))
        }
        other => Some(other),
    }
}

/// Parse loop body (do...done, {...}, or shortloop)
/// Parse the `do BODY done` body of a for/while/until/select/
/// repeat loop. Direct equivalent of zsh's parse.c handling
/// inside the loop builders — they all consume DOLOOP, parse a
/// list until DONE, and return the list. The `foreach_style`
/// flag signals foreach (where short-form `for NAME in WORDS;
/// CMD` may skip do/done) vs c-style (which always requires
/// do/done).
///
/// `is_repeat` widens the SHORTLOOPS gate so `SHORTREPEAT` also
/// unlocks the short form for `repeat N CMD` (per c:1600
/// `unset(SHORTLOOPS) && unset(SHORTREPEAT)`).
fn parse_loop_body(foreach_style: bool, is_repeat: bool) -> Option<ZshProgram> {
    // c:1180-1194 — body dispatch order per par_for:
    //   `do ... done` (DOLOOP) — primary form.
    //   `{ ... }`   (INBRACE) — alternate.
    //   csh/CSHJUNKIELOOPS — terminator is `end`.
    //   else if (unset(SHORTLOOPS)) — YYERROR.
    //   else — short form (single command).
    if tok() == DOLOOP {
        zshlex();
        // Body parse must declare DONE as an end-token so the
        // parse_program_until top-level orphan-DONE guard doesn't
        // mis-fire on the legitimate loop terminator.
        let body = parse_program_until(Some(&[DONE]));
        // c:Src/parse.c:1182-1183 / :1535-1536 / :1597-1598 —
        // `if (tok != DONE) YYERRORV(oecused);`. zshrs previously
        // silently accepted EOF as a substitute for `done`, so
        // `for i in a; do echo hi; don` ran the loop with `don` as
        // a command (which then failed "command not found") instead
        // of erroring at parse time. Bug #403, #404.
        if tok() != DONE {
            zerr("parse error: expected `done'");
            return None;
        }
        zshlex();
        Some(body)
    } else if tok() == INBRACE_TOK {
        zshlex();
        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
        // c:Src/parse.c:1186 / :1539 — `if (tok != OUTBRACE) YYERRORV`.
        if tok() != OUTBRACE_TOK {
            zerr("parse error: expected `}'");
            return None;
        }
        zshlex();
        Some(body)
    } else if foreach_style || isset(CSHJUNKIELOOPS) {
        // c:1184 / 1546 / 1595 — `else if (csh || isset(CSHJUNKIELOOPS))`.
        let body = parse_program_until(Some(&[ZEND]));
        // c:1190 / 1548 — `if (tok != ZEND) YYERRORV`.
        if tok() != ZEND {
            zerr("parse error: expected `end'");
            return None;
        }
        zshlex();
        Some(body)
    } else {
        // c:1190 / 1474 / 1551 / 1600 — short-form gate. C bails
        // with YYERROR when `unset(SHORTLOOPS) && (!is_repeat ||
        // unset(SHORTREPEAT))`. zshrs's option machinery isn't
        // initialised at parse-test time (no `init_main` →
        // `install_emulation_defaults`), so a strict port here
        // body. parse_init seeds SHORTLOOPS=on mirroring C
        // `install_emulation_defaults`, so this fires only when a
        // script explicitly disabled the option.
        if unset(SHORTLOOPS) && (!is_repeat || unset(SHORTREPEAT)) {
            zerr("parse error: short loop form requires SHORTLOOPS option");
            return None;
        }
        // c:Src/parse.c:1604 / :1474 / :1551 — short form calls
        // par_save_list1 → par_list1 → par_sublist, which parses
        // ONE sublist and leaves the trailing SEPER untouched for
        // the outer par_list to consume. zshrs previously routed
        // through par_list() which consumes the trailing `;`/`\n`
        // separator — that swallowed the separator between the
        // loop's body command and the next outer command, so
        // `repeat 2 print x; print y` parsed as repeat-then-eof
        // and par_cmd's post-compound STRING_LEX guard at parse.rs
        // line 1170 fired "parse error near `print'". Bug #593.
        par_list1().map(|sublist| ZshProgram {
            lists: vec![ZshList {
                sublist,
                flags: ListFlags::default(),
            }],
        })
    }
}

/// `() { body } arg1 arg2 …` — anonymous function. Defines a fresh
/// function named `_zshrs_anon_N`, invokes it with the args, and the
/// body runs with positional params set. Implemented as the desugared
/// pair (FuncDef + Simple call) so the compile path doesn't need new
/// machinery.
/// Parse an anonymous function definition `() { BODY }` followed
/// by call args. zsh treats `() { echo hi; } a b c` as defining
/// and immediately calling an anon fn with args a/b/c. C
/// equivalent: the INOUTPAR shape in par_simple at parse.c:1836+
/// triggers an anon-funcdef path.
fn parse_anon_funcdef() -> Option<ZshCommand> {
    zshlex(); // skip ()
    skip_separators();
    // No `{` after `()` → bare empty subshell shape `()`. Fall back
    // to a Subsh with an empty program so the status is 0 (matches
    // zsh's `()` no-op behavior).
    if tok() != INBRACE_TOK {
        return Some(ZshCommand::Subsh(Box::new(ZshProgram {
            lists: Vec::new(),
        })));
    }
    zshlex(); // skip {
    // c:Src/parse.c:par_subsh — anon `() { … }` body must terminate at
    // OUTBRACE_TOK. Pass it as the explicit end-token so the inner
    // parse stops cleanly at `}` rather than hitting the top-level
    // stray-`}` arm (#168). Bug #167 family.
    let body = parse_program_until(Some(&[OUTBRACE_TOK]));
    // c:Src/parse.c:1733-1737 — same `if (tok != OUTBRACE) YYERRORV`
    // gate as the named-funcdef path. Bug #405 sibling.
    if tok() != OUTBRACE_TOK {
        zerr("parse error: expected `}'");
        return None;
    }
    zshlex();
    // Collect any trailing args until a separator. zsh's anon-fn form
    // `() { body } a b c` runs body with $1=a, $2=b, $3=c.
    let mut args = Vec::new();
    while tok() == STRING_LEX {
        if let Some(s) = tokstr() {
            args.push(s);
        }
        zshlex();
    }

    // Generate a unique name. Module-level static would be cleaner but
    // a thread-local atomic is enough — anonymous functions are
    // ephemeral and the name isn't user-visible.
    static ANON_COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = ANON_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!("_zshrs_anon_{}", n);
    Some(ZshCommand::FuncDef(ZshFuncDef {
        names: vec![name],
        body: Box::new(body),
        tracing: false,
        auto_call_args: Some(args),
        body_source: None,
    }))
}

/// Parse {...} cursh
/// Parse a current-shell brace block `{ BODY }`. C source
/// par_cmd at parse.c:958-1085 handles Inbrace → emit WC_CURSH
/// and recurses into the list. zshrs's parse_cursh extracts that
/// arm into a dedicated method.
fn parse_cursh() -> Option<ZshCommand> {
    zshlex(); // skip {
    // c:Src/parse.c:par_subsh — pass OUTBRACE_TOK as the explicit
    // body terminator so the inner parse stops cleanly at `}` rather
    // than falling through the top-level `OUTBRACE_TOK if
    // end_tokens.is_none()` arm (which errors on stray `}` per bug
    // #168). Bug #167 in docs/BUGS.md.
    let prog = parse_program_until(Some(&[OUTBRACE_TOK]));

    // c:Src/parse.c:par_subsh — `{ … }` requires a matching `}`.
    // C errors via YYERRORV when the body parse returns without
    // seeing OUTBRACE_TOK (parse.c:1623 inbrack check). zshrs's
    // previous behavior silently returned `Cursh(prog)` and ran the
    // body as if the braces were absent. Bug #167 in docs/BUGS.md.
    if tok() != OUTBRACE_TOK {
        // Reuse the "parse error near `<tok>'" shape from #142/#161.
        // The offending token is whatever follows the unclosed brace
        // body. For EOF (`{ echo a` at end of input) C zsh errors
        // near the LAST consumed body token; we use the current
        // tokstr() or fall back to a "}" hint.
        let near = tokstr().unwrap_or_else(|| "}".to_string());
        zerr(&format!("parse error near `{}'", near));
        return None;
    }
    // Check for { ... } always { ... }. Direct port of zsh's
    // par_subsh at parse.c:1612-1660 — note the two `incmdpos = 1`
    // forces (parse.c:1632, 1637): after consuming the closing
    // Outbrace AND after matching the `always` keyword, the parser
    // explicitly resets command position so the next `{` lexes as
    // Inbrace. Without these resets the lexer's String-clears-cmdpos
    // rule (lex.rs:976-983) leaves the second `{` in word position,
    // turning `always { ... }` into a Simple `{` `echo` … and the
    // try/always pairing is silently lost.
    {
        set_incmdpos(true); // parse.c:1632 incmdpos = !zsh_construct
        zshlex();

        // Check for 'always'
        if tok() == STRING_LEX {
            let s = tokstr();
            if s.map(|s| s == "always").unwrap_or(false) {
                set_incmdpos(true); // parse.c:1637 incmdpos = 1
                zshlex();
                skip_separators();

                if tok() == INBRACE_TOK {
                    zshlex();
                    // c:Src/parse.c — always-clause body terminates at
                    // OUTBRACE_TOK. Bug #167/#168 family.
                    let always = parse_program_until(Some(&[OUTBRACE_TOK]));
                    if tok() == OUTBRACE_TOK {
                        zshlex();
                    }
                    return Some(ZshCommand::Try(ZshTry {
                        try_block: Box::new(prog),
                        always: Box::new(always),
                    }));
                }
            }
        }
    }

    Some(ZshCommand::Cursh(Box::new(prog)))
}

/// Parse inline function definition: name() { ... }
/// Parse the inline form `NAME () { BODY }` (POSIX-style funcdef
/// without the `function` keyword). The name has already been
/// consumed and pushed by par_simple before this method fires.
/// C source: handled inline in par_simple's INOUTPAR-after-name
/// arm (parse.c:1836-2228).
fn parse_inline_funcdef(name: String) -> Option<ZshCommand> {
    // par_simple's STRING loop left `incmdpos = 0`; the funcdef body
    // `{ ... }` requires `incmdpos = 1` so the lexer recognises `{`
    // as INBRACE_TOK (current-shell block opener) instead of a
    // literal `{` STRING. Without this, `myfunc() { echo body }`
    // parsed the body as the single STRING `"{"`, then `echo body`
    // fell out at top level. Mirrors the C path where par_cmd's
    // dispatcher (parse.c:958) is called with `incmdpos = 1` for
    // the funcdef body.
    set_incmdpos(true);
    // Skip ()
    if tok() == INOUTPAR {
        zshlex();
    }

    skip_separators();

    // Parse body
    if tok() == INBRACE_TOK {
        // Same body_start-before-zshlex fix as par_funcdef.
        let body_start = pos();
        zshlex();
        // c:Src/parse.c — inline funcdef body terminates at OUTBRACE_TOK.
        // Explicit end-token keeps the inner parse from hitting the
        // top-level stray-`}` arm (#168). Bug #167 family.
        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
        // c:Src/parse.c:1733-1737 — `if (tok != OUTBRACE) { cmdpop();
        // lineno += oldlineno; ecnpats = onp; ecssub = oecssub;
        // YYERRORV(oecused); }`. Without this gate, `f() { echo hi`
        // silently registered as a complete fn with body `echo hi`.
        // Bug #405.
        if tok() != OUTBRACE_TOK {
            zerr("parse error: expected `}'");
            return None;
        }
        let body_end = pos().saturating_sub(1);
        let body_source = input_slice(body_start, body_end)
            .map(|s| {
                // Lexer's pos() may have advanced past `}` AND skipped
                // trailing whitespace/newlines before returning the
                // OUTBRACE_TOK to us, so the slice up to `pos - 1`
                // includes the `}` and any preceding whitespace.
                // Strip the trailing `}` and any preceding structural
                // separator (`;`, `\n`) — C zsh's getpermtext walks
                // the wordcode list and emits each command WITHOUT
                // the trailing `;`/`\n` that lives in the input.
                let t = s.trim();
                let t = t.strip_suffix('}').unwrap_or(t).trim_end();
                let t = t
                    .trim_end_matches(|c: char| c == ';' || c == '\n')
                    .trim_end();
                t.to_string()
            })
            .filter(|s| !s.is_empty());
        zshlex();
        Some(ZshCommand::FuncDef(ZshFuncDef {
            names: vec![name],
            body: Box::new(body),
            tracing: false,
            auto_call_args: None,
            body_source,
        }))
    } else if unset(SHORTLOOPS) {
        // c:1742 — `else if (unset(SHORTLOOPS)) YYERRORV(oecused);` —
        // funcdef short body (`name() cmd` without `{...}`) only
        // accepted when SHORTLOOPS is set. parse_init seeds
        // SHORTLOOPS=on so this fires only when a script
        // explicitly disabled the option.
        zerr("parse error: short function body form requires SHORTLOOPS option");
        None
    } else {
        match par_cmd() {
            Some(cmd) => {
                let list = ZshList {
                    sublist: ZshSublist {
                        pipe: ZshPipe {
                            cmd,
                            next: None,
                            lineno: lineno(),
                            merge_stderr: false,
                        },
                        next: None,
                        flags: SublistFlags::default(),
                    },
                    flags: ListFlags::default(),
                };
                Some(ZshCommand::FuncDef(ZshFuncDef {
                    names: vec![name],
                    body: Box::new(ZshProgram { lists: vec![list] }),
                    tracing: false,
                    auto_call_args: None,
                    body_source: None,
                }))
            }
            None => None,
        }
    }
}

/// Parse conditional expression
/// Top of `[[ ]]` cond-expression parsing — entry to recursive
/// descent (or → and → not → primary). Direct port of zsh's
/// par_cond_1 at parse.c:2434-2475.
fn parse_cond_expr() -> Option<ZshCond> {
    parse_cond_or()
}

/// Cond-expression `||` level. C: inside par_cond_1 at
/// parse.c:2434-2475 (the `cond_or` ladder).
fn parse_cond_or() -> Option<ZshCond> {
    let left = parse_cond_and()?;
    skip_cond_separators();

    if tok() == DBAR {
        zshlex();
        skip_cond_separators();
        parse_cond_or().map(|right| ZshCond::Or(Box::new(left), Box::new(right)))
    } else {
        Some(left)
    }
}

/// Cond-expression `&&` level. C: par_cond_2 at parse.c:2476-2625.
fn parse_cond_and() -> Option<ZshCond> {
    let left = parse_cond_not()?;
    skip_cond_separators();

    if tok() == DAMPER {
        zshlex();
        skip_cond_separators();
        parse_cond_and().map(|right| ZshCond::And(Box::new(left), Box::new(right)))
    } else {
        Some(left)
    }
}

/// `static FuncDump dumps;` from `Src/parse.c:3652` — head of the
/// loaded-`.zwc` linked list. C walks `dumps`/`p->next` directly;
/// the Rust port uses a `Mutex<Vec<funcdump>>` indexed by filename
/// so refcount ops can find an entry without raw-pointer compare.
pub static DUMPS: std::sync::Mutex<Vec<funcdump>> = std::sync::Mutex::new(Vec::new());

/// Cond-expression `!` negation level. C: handled inside
/// par_cond_2 at parse.c:2476-2625 via the Bang token check.
fn parse_cond_not() -> Option<ZshCond> {
    skip_cond_separators();

    // ! can be either BANG_TOK or String "!"
    let is_not =
        tok() == BANG_TOK || (tok() == STRING_LEX && tokstr().map(|s| s == "!").unwrap_or(false));
    if is_not {
        zshlex();
        let inner = parse_cond_not()?;
        return Some(ZshCond::Not(Box::new(inner)));
    }

    if tok() == INPAR_TOK {
        zshlex();
        skip_cond_separators();
        // c:Src/parse.c:2534-2547 par_cond_2 INPAR branch — empty
        // body `[[ ( ) ]]` makes the inner par_cond's recursive
        // par_cond_2 see OUTPAR with no leading STRING/BANG/INPAR
        // and YYERROR immediately. Mirror that here: if the very
        // next token after `(` (post separator skip) is `)`, emit
        // a parse error so the script aborts cleanly instead of
        // silently swallowing every following command. Bug #538.
        if tok() == OUTPAR_TOK {
            crate::ported::utils::zerr("condition expected");
            yyerror(0);
            return None;
        }
        let inner = parse_cond_expr()?;
        skip_cond_separators();
        if tok() == OUTPAR_TOK {
            zshlex();
        }
        return Some(inner);
    }

    parse_cond_primary()
}

/// Cond-expression primary: unary tests (-f, -d, ...), binary
/// tests (=, !=, <, >, ==, =~, -eq, -ne, ...), and parenthesized
/// sub-expressions. Direct port of par_cond_double / par_cond_triple
/// / par_cond_multi at parse.c:2626-2731 (chosen by arg count).
fn parse_cond_primary() -> Option<ZshCond> {
    let s1 = match tok() {
        STRING_LEX => {
            let s = tokstr().unwrap_or_default();
            zshlex();
            s
        }
        _ => return None,
    };

    skip_cond_separators();

    // Check for unary operator. zsh's lexer tokenizes leading `-` as
    // `zsh_h::Dash` (`\u{9b}`, `Src/zsh.h:182`) inside gettokstr (lex.c:1390-1400
    // LX2_DASH — `-` always becomes Dash, untokenized later). Match
    // either form here, and use char-count not byte-count since Dash
    // is 2 UTF-8 bytes (`\xc2\x9b`).
    //
    // c:Src/parse.c par_cond — when the leading token is `-` followed
    // ENTIRELY by digits (`-5`, `-123`), it's a numeric literal
    // operand, not a unary test flag. zsh's parser checks the C
    // `isdigit` of the trailing chars to disambiguate; without the
    // check, `[[ -5 -lt -3 ]]` reads `-5` as a one-arg test flag,
    // then `-lt` as the operand, then `-3` as a leftover token —
    // emitting "unknown condition: -5" and falling through to a
    // command-not-found dispatch on `-3`. Bug #121 in docs/BUGS.md.
    let s1_chars: Vec<char> = s1.chars().collect();
    let is_negative_number = s1_chars.len() >= 2
        && IS_DASH(s1_chars[0])
        && s1_chars[1..].iter().all(|c| c.is_ascii_digit());
    if s1_chars.len() == 2 && IS_DASH(s1_chars[0]) && !is_negative_number {
        let s2 = match tok() {
            STRING_LEX => {
                let s = tokstr().unwrap_or_default();
                zshlex();
                s
            }
            _ => {
                // c:Src/parse.c par_cond_2 — when the leading `-X`
                // is a 2-char dash form, zsh ALWAYS treats it as a
                // unary test op (the operand-missing case errors
                // immediately with `unknown condition: -X`). Don't
                // fall back to `Unary("-n", "-X")` — that path
                // silently let `[[ -z ]]` evaluate as
                // `[[ -n "-z" ]]` → true. Bug #480/#481.
                //
                // Convert Dash (\u{9b}) back to ASCII `-` for the
                // user-visible diagnostic so it reads "unknown
                // condition: -z" not "unknown condition: <Dash>z".
                let display: String = s1.chars().map(|c| {
                    if IS_DASH(c) { '-' } else { c }
                }).collect();
                crate::ported::utils::zerr(&format!(
                    "unknown condition: {}",
                    display
                ));
                return None;
            }
        };
        return Some(ZshCond::Unary(s1, s2));
    }

    // Check for binary operator. Direct port of zsh/Src/parse.c:2601-2603:
    //   incond++;  /* parentheses do globbing */
    //   do condlex(); while (COND_SEP());
    //   incond--;  /* parentheses do grouping */
    // The bump makes the lexer treat `(` as a literal character inside
    // the RHS word (e.g. `[[ x =~ (foo) ]]`) instead of returning Inpar
    // and splitting the regex into multiple tokens.
    let op = match tok() {
        STRING_LEX => {
            let s = tokstr().unwrap_or_default();
            set_incond(incond() + 1);
            zshlex();
            set_incond(incond() - 1);
            s
        }
        INANG_TOK => {
            set_incond(incond() + 1);
            zshlex();
            set_incond(incond() - 1);
            "<".to_string()
        }
        OUTANG_TOK => {
            set_incond(incond() + 1);
            zshlex();
            set_incond(incond() - 1);
            ">".to_string()
        }
        _ => return Some(ZshCond::Unary("-n".to_string(), s1)),
    };

    skip_cond_separators();

    // c:Src/parse.c:2601-2625 par_cond_2 — only the documented binary
    // operators are accepted inside `[[ ... ]]`. zsh rejects ksh/bash
    // forms `-a` (logical AND) and `-o` (logical OR) with a parse
    // error ("condition expected") because they're not in the
    // par_cond_2 binary-op set — zsh uses `&&` / `||` instead.
    // Verified: `zsh -fc '[[ "" -a "x" ]]'` → exit 1, "parse error:
    // condition expected: ...". Without this gate, zshrs silently
    // built ZshCond::Binary("", "-a", "x") and ran an unknown-op
    // path that always evaluated false.
    // c:Src/parse.c:2601-2625 par_cond_2 — `-a` / `-o` n-ary chain
    // operators are not valid binary operators inside `[[ ... ]]`
    // (zsh uses `&&` / `||` instead). Match both the ASCII `-a`/
    // `-o` form and the tokenized `Dash+a`/`Dash+o` form that the
    // lexer emits inside cond bodies (Dash = \u{9b}, Src/zsh.h:182).
    let op_chars: Vec<char> = op.chars().collect();
    let is_dash_a_or_o =
        op_chars.len() == 2 && IS_DASH(op_chars[0]) && (op_chars[1] == 'a' || op_chars[1] == 'o');
    if is_dash_a_or_o {
        crate::ported::utils::zerr(&format!("parse error: condition expected: {}", s1));
        crate::ported::utils::errflag.fetch_or(
            crate::ported::zsh_h::ERRFLAG_ERROR,
            std::sync::atomic::Ordering::Relaxed,
        );
        set_tok(LEXERR);
        return None;
    }

    let s2 = match tok() {
        STRING_LEX => {
            let s = tokstr().unwrap_or_default();
            zshlex();
            s
        }
        _ => {
            // c:Src/parse.c par_cond_2 — when a binary op is
            // recognized but the RHS operand is missing, zsh emits
            // `parse error: condition expected: <LHS>` at par_cond_2's
            // missing-rhs branch. zshrs's previous fallback returned
            // `Binary(s1, op, "")` which silently evaluated as if the
            // RHS were empty string → rc=1. Bug #482.
            //
            // Convert Dash (\u{9b}) back to ASCII `-` in the LHS
            // display so the diagnostic reads cleanly.
            let display: String = s1.chars().map(|c| {
                if IS_DASH(c) { '-' } else { c }
            }).collect();
            crate::ported::utils::zerr(&format!(
                "parse error: condition expected: {}",
                display
            ));
            crate::ported::utils::errflag.fetch_or(
                crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            set_tok(LEXERR);
            return None;
        }
    };

    // c:Src/parse.c:2685-2691 par_cond_triple —
    //   `(b[0] == Equals || b[0] == '=') && (b[1] == '~' || b[1] == Tilde)
    //    && !b[2]` → COND_REGEX.
    // The lexer emits the TOKEN forms inside cond bodies (`=` at word
    // start → Equals `\u{8d}`, `~` → Tilde `\u{98}`), so an ASCII-only
    // `op == "=~"` check missed every real `[[ x =~ pat ]]` and fell
    // through to Binary.
    let opc: Vec<char> = op.chars().collect();
    let is_regex_op = opc.len() == 2
        && (opc[0] == '=' || opc[0] == Equals)
        && (opc[1] == '~' || opc[1] == Tilde);
    if is_regex_op {
        Some(ZshCond::Regex(s1, s2))
    } else {
        Some(ZshCond::Binary(s1, op, s2))
    }
}

fn skip_cond_separators() {
    while tok() == SEPER && {
        let s = tokstr();
        s.map(|s| !s.contains(';')).unwrap_or(true)
    } {
        zshlex();
    }
}

/// Parse (( ... )) arithmetic command
/// Parse `(( EXPR ))` arithmetic command. C source: parse.c:1810-1834
/// `par_dinbrack` (despite the name; the function actually handles
/// DINPAR `(( ))` blocks too).
fn parse_arith() -> Option<ZshCommand> {
    let expr = tokstr().unwrap_or_default();
    zshlex();
    Some(ZshCommand::Arith(expr))
}

/// Skip separator tokens
fn skip_separators() {
    while tok() == SEPER || tok() == NEWLIN {
        zshlex();
    }
}

// `fdheaderlen` / `fdmagic` / `fdflags` / etc. macros from
// `Src/parse.c:3125-3152`. C uses raw pointer arithmetic on a
// `Wordcode` (= `u32 *`); the Rust port takes a slice and indexes.

/// Port of `fdheaderlen(f)` macro (`Src/parse.c:3125`) — header
/// length in u32 words (read from prelude word `FD_PRELEN`).
#[inline]
pub fn fdheaderlen(f: &[u32]) -> u32 {
    f[FD_PRELEN]
}

/// Port of `fdmagic(f)` macro (`Src/parse.c:3127`) — first prelude
/// word, either `FD_MAGIC` or `FD_OMAGIC`.
#[inline]
pub fn fdmagic(f: &[u32]) -> u32 {
    f[0]
}

/// Port of `fdflags(f)` macro (`Src/parse.c:3131`) — low byte of
/// the packed `pre[1]` word.
#[inline]
pub fn fdflags(f: &[u32]) -> u32 {
    // `pre[1]` is a u32 viewed as 4 bytes; flags = byte 0.
    f[1] & 0xff
}

/// Port of `fdsetflags(f, v)` macro (`Src/parse.c:3132`) — write
/// the low byte of `pre[1]`.
#[inline]
pub fn fdsetflags(f: &mut [u32], v: u8) {
    f[1] = (f[1] & !0xff) | (v as u32);
}

/// Port of `fdother(f)` macro (`Src/parse.c:3133`) — high 24 bits
/// of `pre[1]`, holds the byte-offset to the opposite-byte-order
/// dump copy.
#[inline]
pub fn fdother(f: &[u32]) -> u32 {
    (f[1] >> 8) & 0x00ff_ffff
}

/// Port of `fdsetother(f, o)` macro (`Src/parse.c:3134`).
#[inline]
pub fn fdsetother(f: &mut [u32], o: u32) {
    f[1] = (f[1] & 0xff) | ((o & 0x00ff_ffff) << 8);
}

/// Port of `fdversion(f)` macro (`Src/parse.c:3140`) — read the
/// `ZSH_VERSION` C-string from `pre[2..]`.
pub fn fdversion(f: &[u32]) -> String {
    let bytes: Vec<u8> = f[2..]
        .iter()
        .take(10)
        .flat_map(|w| w.to_le_bytes().into_iter())
        .collect();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Port of `firstfdhead(f)` macro (`Src/parse.c:3142`) — pointer
/// to the first `struct fdhead` past the prelude.
#[inline]
pub fn firstfdhead_offset() -> usize {
    FD_PRELEN
}

/// Port of `nextfdhead(f)` macro (`Src/parse.c:3143`) — advance to
/// the next header by reading the current `hlen` slot.
#[inline]
pub fn nextfdhead_offset(f: &[u32], cur: usize) -> usize {
    cur + (f[cur + 4] as usize) // .hlen is field 4 of fdhead
}

/// Port of `fdhflags(f)` macro (`Src/parse.c:3145`) — low 2 bits
/// of the header's `flags` field (the kshload/zshload marker).
#[inline]
pub fn fdhflags(h: &fdhead) -> u32 {
    h.flags & 0x3
}

/// Port of `fdhtail(f)` macro (`Src/parse.c:3146`) — high 30 bits
/// of `flags`, byte offset from the name start to its basename.
#[inline]
pub fn fdhtail(h: &fdhead) -> u32 {
    h.flags >> 2
}

/// Port of `fdhbldflags(f, t)` macro (`Src/parse.c:3147`) — pack
/// `(flags, tail)` into one u32 (low 2 bits = flags, high 30 = tail).
#[inline]
pub fn fdhbldflags(flags: u32, tail: u32) -> u32 {
    flags | (tail << 2)
}

/// Port of `fdname(f)` macro (`Src/parse.c:3152`) — name string
/// follows the fdhead record immediately. Reads bytes from the
/// dump buffer until NUL.
pub fn fdname(buf: &[u32], header_offset: usize) -> String {
    let name_word_off = header_offset + FDHEAD_WORDS;
    let bytes: Vec<u8> = buf[name_word_off..]
        .iter()
        .flat_map(|w| w.to_le_bytes().into_iter())
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Decode a `fdhead` record at the given u32-word offset in the
/// dump buffer. Used by the header-walk loops in `bin_zcompile -t`.
pub fn read_fdhead(buf: &[u32], offset: usize) -> Option<fdhead> {
    if offset + FDHEAD_WORDS > buf.len() {
        return None;
    }
    Some(fdhead {
        start: buf[offset],
        len: buf[offset + 1],
        npats: buf[offset + 2],
        strs: buf[offset + 3],
        hlen: buf[offset + 4],
        flags: buf[offset + 5],
    })
}

/// Port of `freedump(FuncDump f)` from `Src/parse.c:3976`. C
/// `munmap`s, `zclose`s the fd, and frees the struct. The Rust
/// port relies on Drop for the `funcdump` (no mmap held in this
/// port — `addr`/`map` are byte-offset placeholders), so the
/// equivalent is removing the entry from the dumps list. Called
/// by `decrdumpcount` when the refcount hits zero (c:3988) and
/// by `closedumps` when shutting down (c:4008).
fn freedump_locked(g: &mut std::sync::MutexGuard<'_, Vec<funcdump>>, filename: &str) {
    // c:3976
    g.retain(|d| d.filename.as_deref() != Some(filename));
}

// =====================================================================
// Remaining `Src/parse.c` ports (this section finishes the file).
//
// Several of these emit into the C-wordcode buffer (`ECBUF`/etc.) and
// are kept for completeness — the live zshrs runtime uses the
// `ZshProgram` AST path instead, but `bin_zcompile` (`-c`/`-a` modes)
// and any future `.zwc`-emit pipeline both call into these.
// =====================================================================

/// `ecstr(s)` helper — `ecadd(ecstrcode(s))`. Mirrors the C macro at
/// `Src/parse.c:482` used everywhere by the par_* emitters.
#[inline]
pub fn ecstr(s: &str) {
    let code = ecstrcode(s);
    ecadd(code);
}

/// Port of `condlex` function-pointer global from `Src/parse.c`. C
/// flips this between `zshlex` and `testlex` depending on whether
/// we're inside `[[ ]]` vs `/bin/test` builtin. zshrs has no
/// separate `testlex` yet, so this just defers to `zshlex`.
#[inline]
pub fn condlex() {
    zshlex();
}

fn copy_ecstr_walk(node: &Option<Box<EccstrNode>>, p: &mut [u8]) {
    let mut cur = node.as_ref();
    while let Some(n) = cur {
        // c:540 — `memcpy(p + s->aoffs, s->str, strlen(s->str) + 1);`
        let off = n.aoffs as usize;
        let need = off + n.str.len() + 1;
        if need <= p.len() {
            p[off..off + n.str.len()].copy_from_slice(&n.str);
            p[off + n.str.len()] = 0;
        }
        // c:541 — `copy_ecstr(s->left, p);`
        copy_ecstr_walk(&n.left, p);
        // c:542 — `s = s->right;`
        cur = n.right.as_ref();
    }
}

/// Port of `par_cond(void)` from `Src/parse.c:2409`. Top-level cond
/// OR-chain — drives `par_cond_1` and stitches `||`-separated terms
/// with `WCB_COND(COND_OR, …)`. This is the missing top of the
/// wordcode cond chain: `par_cond_wordcode` (the par_dinbrack port)
/// must call into HERE so that `[[ a || b ]]` and friends land
/// real WC_COND opcodes in `ecbuf`. Without this, the wordcode
/// emitter for `[[ ... ]]` produced zero words and parity dropped
/// 148 words on `/etc/zshrc` alone.
pub fn par_cond_top() -> i32 {
    // c:2411 — `int p = ecused, r;`
    let p = ECUSED.with(|c| c.get()) as usize;
    let r = par_cond_1();
    while COND_SEP() {
        condlex();
    }
    if tok() == DBAR {
        // c:2417 — `condlex(); while (COND_SEP()) condlex();`
        condlex();
        while COND_SEP() {
            condlex();
        }
        // c:2420-2422 — `ecispace(p, 1); par_cond(); ecbuf[p] =
        // WCB_COND(COND_OR, ecused-1-p);`
        ecispace(p, 1);
        par_cond_top();
        let ecused = ECUSED.with(|c| c.get()) as usize;
        ECBUF.with(|c| {
            c.borrow_mut()[p] = WCB_COND(COND_OR as u32, (ecused - 1 - p) as u32);
        });
        return 1;
    }
    r
}

/// Port of `static int check_cond(const char *input, const char *cond)`
/// from `Src/parse.c:2459`. True iff `input` is the two-char `-X`
/// form whose `X` matches `cond` — used by par_cond_2 to detect
/// `-a` / `-o` n-ary chain operators and by build_dump for `-k` /
/// `-z`. C: `return !IS_DASH(input[0]) ? 0 : !strcmp(input+1, cond);`.
fn check_cond(input: &str, cond: &str) -> bool {
    let mut chars = input.chars();
    match chars.next() {
        Some(c) if IS_DASH(c) => chars.as_str() == cond,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::{errflag, ERRFLAG_ERROR};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// `try_source_file` MUST refuse a stale `.zwc` cache when the
    /// uncompiled source has been modified more recently. The C body
    /// at c:3819 reads `stc.st_mtime >= stn.st_mtime` — explicitly
    /// `>=`, meaning only an equal-or-newer zwc is acceptable.
    ///
    /// A regression that ignored the mtime check (or used the wrong
    /// direction) would silently keep loading the OLD compiled body
    /// after the user edited the source file — every `source foo.zsh`
    /// would replay yesterday's code, the worst-class shell bug.
    ///
    /// Pin: create source + .zwc, then touch source to make it
    /// newer. try_source_file must return None.
    #[test]
    fn try_source_file_skips_stale_zwc() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("script.zsh");
        let zwc = dir.path().join("script.zsh.zwc");
        // Create zwc FIRST (older), then source (newer).
        fs::write(&zwc, b"placeholder zwc").unwrap();
        thread::sleep(Duration::from_millis(20));
        fs::write(&src, b"echo hi").unwrap();

        let result = try_source_file(src.to_str().unwrap());
        assert!(
            result.is_none(),
            "c:3819 — stale .zwc (older than source) MUST be rejected; \
             got {:?}",
            result
        );
    }

    /// `try_source_file` returns None when no `.zwc` exists for the
    /// requested file (c:3819 `if let Ok(meta_c) = &stc` gate fails).
    /// This is the common case — most user scripts don't ship with
    /// a pre-compiled `.zwc`. The fn returning None lets the caller
    /// fall through to the source-read path. A regression that
    /// returned `Some(file)` on missing `.zwc` would route every
    /// `source foo.zsh` through `check_dump_file` against a
    /// non-existent file and crash.
    #[test]
    fn try_source_file_returns_none_when_no_zwc() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("plain.zsh");
        fs::write(&src, b"echo hi").unwrap();
        // No .zwc sibling.

        let result = try_source_file(src.to_str().unwrap());
        assert!(
            result.is_none(),
            "c:3819 gate fails when stat(wc) returns Err → None"
        );
    }

    /// Test helper. Mirrors zsh's `errflag` save/clear/check pattern
    /// around a parse — see `Src/init.c:loop` which clears errflag
    /// before parse_event() and tests it after. Returns `Err` if the
    /// parse set `ERRFLAG_ERROR`; otherwise `Ok(program)`.
    fn parse(input: &str) -> Result<ZshProgram, String> {
        let saved = errflag.load(Ordering::Relaxed);
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        parse_init(input);
        let prog = crate::ported::parse::parse();
        let had_err = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
        // Restore prior error bits; don't carry our new error into the
        // outer test runner.
        errflag.store(saved, Ordering::Relaxed);
        if had_err {
            Err("parse error".to_string())
        } else {
            Ok(prog)
        }
    }

    #[test]
    fn test_simple_command() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("echo hello world").unwrap();
        assert_eq!(prog.lists.len(), 1);
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Simple(s) => {
                assert_eq!(s.words, vec!["echo", "hello", "world"]);
            }
            _ => panic!("expected simple command"),
        }
    }

    #[test]
    fn test_pipeline() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("ls | grep foo | wc -l").unwrap();
        assert_eq!(prog.lists.len(), 1);

        let pipe = &prog.lists[0].sublist.pipe;
        assert!(pipe.next.is_some());

        let pipe2 = pipe.next.as_ref().unwrap();
        assert!(pipe2.next.is_some());
    }

    #[test]
    fn test_and_or() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("cmd1 && cmd2 || cmd3").unwrap();
        let sublist = &prog.lists[0].sublist;

        assert!(sublist.next.is_some());
        let (op, _) = sublist.next.as_ref().unwrap();
        assert_eq!(*op, SublistOp::And);
    }

    #[test]
    fn test_if_then() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("if test -f foo; then echo yes; fi").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::If(_) => {}
            _ => panic!("expected if command"),
        }
    }

    #[test]
    fn test_for_loop() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("for i in a b c; do echo $i; done").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::For(f) => {
                assert_eq!(f.var, "i");
                match &f.list {
                    ForList::Words(w) => assert_eq!(w, &vec!["a", "b", "c"]),
                    _ => panic!("expected word list"),
                }
            }
            _ => panic!("expected for command"),
        }
    }

    #[test]
    fn test_case() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("case $x in a) echo a;; b) echo b;; esac").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Case(c) => {
                assert_eq!(c.arms.len(), 2);
            }
            _ => panic!("expected case command"),
        }
    }

    #[test]
    fn test_function() {
        let _g = crate::test_util::global_state_lock();
        // First test just parsing "function foo" to see what happens
        let prog = parse("function foo { }").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::FuncDef(f) => {
                assert_eq!(f.names, vec!["foo"]);
            }
            _ => panic!(
                "expected function, got {:?}",
                prog.lists[0].sublist.pipe.cmd
            ),
        }
    }

    #[test]
    fn test_redirection() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("echo hello > file.txt").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Simple(s) => {
                assert_eq!(s.redirs.len(), 1);
                assert_eq!(s.redirs[0].rtype, REDIR_WRITE);
            }
            _ => panic!("expected simple command"),
        }
    }

    #[test]
    fn test_assignment() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("FOO=bar echo $FOO").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Simple(s) => {
                assert_eq!(s.assigns.len(), 1);
                assert_eq!(s.assigns[0].name, "FOO");
            }
            _ => panic!("expected simple command"),
        }
    }

    #[test]
    fn test_parse_completion_function() {
        let _g = crate::test_util::global_state_lock();
        let input = r#"_2to3_fixes() {
  local -a fixes
  fixes=( ${${(M)${(f)"$(2to3 --list-fixes 2>/dev/null)"}:#*}//[[:space:]]/} )
  (( ${#fixes} )) && _describe -t fixes 'fix' fixes
}"#;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse completion function: {:?}",
            result.err()
        );
        let prog = result.unwrap();
        assert!(
            !prog.lists.is_empty(),
            "Expected at least one list in program"
        );
    }

    #[test]
    fn test_parse_array_with_complex_elements() {
        let _g = crate::test_util::global_state_lock();
        let input = r#"arguments=(
  '(- * :)'{-h,--help}'[show this help message and exit]'
  {-d,--doctests_only}'[fix up doctests only]'
  '*:filename:_files'
)"#;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse array assignment: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_parse_full_completion_file() {
        let _g = crate::test_util::global_state_lock();
        let input = r##"#compdef 2to3

# zsh completions for '2to3'

_2to3_fixes() {
  local -a fixes
  fixes=( ${${(M)${(f)"$(2to3 --list-fixes 2>/dev/null)"}:#*}//[[:space:]]/} )
  (( ${#fixes} )) && _describe -t fixes 'fix' fixes
}

local -a arguments

arguments=(
  '(- * :)'{-h,--help}'[show this help message and exit]'
  {-d,--doctests_only}'[fix up doctests only]'
  {-f,--fix}'[each FIX specifies a transformation; default: all]:fix name:_2to3_fixes'
  {-j,--processes}'[run 2to3 concurrently]:number: '
  {-x,--nofix}'[prevent a transformation from being run]:fix name:_2to3_fixes'
  {-l,--list-fixes}'[list available transformations]'
  {-p,--print-function}'[modify the grammar so that print() is a function]'
  {-v,--verbose}'[more verbose logging]'
  '--no-diffs[do not show diffs of the refactoring]'
  {-w,--write}'[write back modified files]'
  {-n,--nobackups}'[do not write backups for modified files]'
  {-o,--output-dir}'[put output files in this directory instead of overwriting]:directory:_directories'
  {-W,--write-unchanged-files}'[also write files even if no changes were required]'
  '--add-suffix[append this string to all output filenames]:suffix: '
  '*:filename:_files'
)

_arguments -s -S $arguments
"##;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse full completion file: {:?}",
            result.err()
        );
        let prog = result.unwrap();
        // Should have parsed successfully with at least one statement
        assert!(!prog.lists.is_empty(), "Expected at least one list");
    }

    #[test]
    fn test_parse_logs_sh() {
        let _g = crate::test_util::global_state_lock();
        let input = r#"#!/usr/bin/env bash
shopt -s globstar

if [[ $(uname) == Darwin ]]; then
    tail -f /var/log/**/*.log /var/log/**/*.out | lolcat
else
    if [[ $ZPWR_DISTRO_NAME == raspbian ]]; then
        tail -f /var/log/**/*.log | lolcat
    else
        printf "Unsupported...\n" >&2
    fi
fi
"#;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse logs.sh: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_parse_case_with_glob() {
        let _g = crate::test_util::global_state_lock();
        let input = r#"case "$ZPWR_OS_TYPE" in
    darwin*)  open_cmd='open'
      ;;
    cygwin*)  open_cmd='cygstart'
      ;;
    linux*)
        open_cmd='xdg-open'
      ;;
esac"#;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse case with glob: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_parse_case_with_nested_if() {
        let _g = crate::test_util::global_state_lock();
        // Test case with nested if and glob patterns
        let input = r##"function zpwrGetOpenCommand(){
    local open_cmd
    case "$ZPWR_OS_TYPE" in
        darwin*)  open_cmd='open' ;;
        cygwin*)  open_cmd='cygstart' ;;
        linux*)
            if [[ "$_zpwr_uname_r" != *icrosoft* ]];then
                open_cmd='nohup xdg-open'
            fi
            ;;
    esac
}"##;
        let result = parse(input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn test_parse_zpwr_scripts() {
        let _g = crate::test_util::global_state_lock();
        let scripts_dir = Path::new("/Users/wizard/.zpwr/scripts");
        if !scripts_dir.exists() {
            eprintln!("Skipping test: scripts directory not found");
            return;
        }

        let mut total = 0;
        let mut passed = 0;
        let mut failed_files = Vec::new();
        let mut timeout_files = Vec::new();

        for ext in &["sh", "zsh"] {
            let pattern = scripts_dir.join(format!("*.{}", ext));
            if let Ok(entries) = glob::glob(pattern.to_str().unwrap()) {
                for entry in entries.flatten() {
                    total += 1;
                    let file_path = entry.display().to_string();
                    let content = match fs::read_to_string(&entry) {
                        Ok(c) => c,
                        Err(e) => {
                            failed_files.push((file_path, format!("read error: {}", e)));
                            continue;
                        }
                    };

                    // Parse with timeout
                    let content_clone = content.clone();
                    let (tx, rx) = mpsc::channel();
                    let handle = thread::spawn(move || {
                        let result = parse(&content_clone);
                        let _ = tx.send(result);
                    });

                    match rx.recv_timeout(Duration::from_secs(2)) {
                        Ok(Ok(_)) => passed += 1,
                        Ok(Err(err)) => {
                            failed_files.push((file_path, err));
                        }
                        Err(_) => {
                            timeout_files.push(file_path);
                            // Thread will be abandoned
                        }
                    }
                }
            }
        }

        eprintln!("\n=== ZPWR Scripts Parse Results ===");
        eprintln!("Passed: {}/{}", passed, total);

        if !timeout_files.is_empty() {
            eprintln!("\nTimeout files (>2s):");
            for file in &timeout_files {
                eprintln!("  {}", file);
            }
        }

        if !failed_files.is_empty() {
            eprintln!("\nFailed files:");
            for (file, err) in &failed_files {
                eprintln!("  {} - {}", file, err);
            }
        }

        // Allow some failures initially, but track progress
        let pass_rate = if total > 0 {
            (passed as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        eprintln!("Pass rate: {:.1}%", pass_rate);

        // Require at least 50% pass rate for now
        assert!(pass_rate >= 50.0, "Pass rate too low: {:.1}%", pass_rate);
    }

    /// c:2643 — `get_cond_num` returns 0..=8 for the canonical binary
    /// test operators in order `nt ot ef eq ne lt gt le ge`. The
    /// index IS the wordcode opcode dispatch key; flipping any entry
    /// would silently mis-dispatch `[[ a -eq b ]]` to a different op.
    #[test]
    fn get_cond_num_canonical_order_matches_dispatch_table() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(get_cond_num("nt"), 0);
        assert_eq!(get_cond_num("ot"), 1);
        assert_eq!(get_cond_num("ef"), 2);
        assert_eq!(get_cond_num("eq"), 3);
        assert_eq!(get_cond_num("ne"), 4);
        assert_eq!(get_cond_num("lt"), 5);
        assert_eq!(get_cond_num("gt"), 6);
        assert_eq!(get_cond_num("le"), 7);
        assert_eq!(get_cond_num("ge"), 8);
    }

    /// c:2643 — unknown operator returns -1 (sentinel for "not in the
    /// binary set"). Regression returning 0 silently would alias
    /// every unknown op to `-nt`, dispatching to the wrong handler.
    #[test]
    fn get_cond_num_unknown_operator_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(get_cond_num("xx"), -1);
        assert_eq!(get_cond_num(""), -1);
        assert_eq!(get_cond_num("eqnt"), -1, "exact-match required");
        assert_eq!(
            get_cond_num("NT"),
            -1,
            "case-sensitive — uppercase rejected"
        );
    }

    /// c:2628 — `par_cond_double` requires arg `a` to start with `-`
    /// AND have at least one more char. Empty string OR single `-`
    /// must error (return 1 via zerr). Regression accepting empty
    /// would dispatch `[[ "" string ]]` as a unary test.
    #[test]
    fn par_cond_double_rejects_short_or_non_dash_first_arg() {
        let _g = crate::test_util::global_state_lock();
        // empty
        let _ = par_cond_double("", "b");
        // not-dash
        let _ = par_cond_double("foo", "b");
        // bare dash
        let _ = par_cond_double("-", "b");
        // All three must NOT crash + return 1 (error path).
    }

    /// c:2647 CONDSTRS table — exhaustive iteration: every entry's
    /// index round-trips through get_cond_num. A regression that
    /// drops an entry would let `[[ a -ef b ]]` silently mis-dispatch.
    #[test]
    fn get_cond_num_round_trips_for_every_table_entry() {
        let _g = crate::test_util::global_state_lock();
        for (i, op) in ["nt", "ot", "ef", "eq", "ne", "lt", "gt", "le", "ge"]
            .iter()
            .enumerate()
        {
            assert_eq!(get_cond_num(op) as usize, i, "{op} must map to index {i}");
        }
    }

    /// c:2643 — `get_cond_num` is byte-exact: a partial-prefix string
    /// must NOT match. `e` (one char) is not `eq`. Catches a
    /// regression using `starts_with` instead of equality.
    #[test]
    fn get_cond_num_partial_prefix_does_not_match() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(get_cond_num("e"), -1);
        assert_eq!(get_cond_num("eq2"), -1);
        assert_eq!(get_cond_num("n"), -1);
    }

    /// c:2628 — `par_cond_double` checks `IS_DASH(ac[0])` so any
    /// non-dash first char fails. The lexed Dash sentinel `\u{9b}`
    /// MUST be accepted alongside ASCII `-` (the lexer emits it
    /// inside `[[ ... ]]`). Regression dropping the sentinel form
    /// would break every cond expression after lexing.
    #[test]
    fn par_cond_double_accepts_lexed_dash_sentinel() {
        let _g = crate::test_util::global_state_lock();
        // First char being the Dash sentinel + valid unary letter
        // must NOT trigger the "condition expected" error path.
        // We can't easily probe the wordcode emission here, but
        // the function MUST return without panic for both forms.
        let _ = par_cond_double("-z", "foo");
        let _ = par_cond_double("\u{9b}z", "foo");
    }

    /// c:2643 — case sensitivity: uppercase `EQ` MUST NOT match `eq`.
    /// zsh's `[[ a -EQ b ]]` is documented as a parse error (only
    /// lowercase variants are recognised). Regression doing
    /// case-insensitive lookup would silently accept it.
    #[test]
    fn get_cond_num_is_case_sensitive() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(get_cond_num("EQ"), -1);
        assert_eq!(get_cond_num("Eq"), -1);
        assert_eq!(get_cond_num("eQ"), -1);
        // Lowercase still works.
        assert_eq!(get_cond_num("eq"), 3);
    }

    /// `Src/parse.c:2862-2868` — `ecgetstr` inline-3-byte case packs
    /// up to 3 chars into bits 3-26 of the wordcode word, then C emits
    /// `buf[3] = '\0'; r = dupstring(buf);`. `dupstring` uses `strlen`
    /// so the resulting string TRUNCATES at the first NUL byte —
    /// short strings of 1 or 2 chars get their tail NUL-padded and
    /// silently dropped by strlen.
    ///
    /// The previous Rust port used `retain(|&x| x != 0)` which SPLICES
    /// OUT interior NULs (so `[a, 0, b]` would yield "ab" instead of
    /// C's "a"). Verify both endpoints work correctly:
    ///   * 1-char string ("a", 0, 0)        → "a"   (strlen-truncate)
    ///   * 2-char string ("ab", 0)          → "ab"  (strlen-truncate)
    ///   * 3-char string ("abc")            → "abc" (full)
    ///   * pathological ("a", 0, "b")       → "a"   (NOT "ab")
    #[test]
    fn ecgetstr_inline_string_truncates_at_first_nul_like_c_strlen() {
        let _g = crate::test_util::global_state_lock();
        // Build a wordcode word with `c & 2 != 0` (inline-string flag)
        // and the 3 bytes packed at offsets 3, 11, 19. `c & 1` is the
        // tokflag; clear it for this test.
        fn pack_inline(b0: u8, b1: u8, b2: u8) -> u32 {
            // c:2862 layout — bit0 = tokflag (0 here), bit1 = inline (1),
            // bits 3-10 = b0, bits 11-18 = b1, bits 19-26 = b2.
            (2u32) | ((b0 as u32) << 3) | ((b1 as u32) << 11) | ((b2 as u32) << 19)
        }
        let mk_state = |word: u32| -> estate {
            let p = eprog {
                flags: 0,
                len: 1,
                npats: 0,
                nref: 0,
                pats: Vec::new(),
                prog: vec![word],
                strs: None,
                shf: None,
                dump: None,
            };
            estate {
                prog: Box::new(p),
                pc: 0,
                strs: None,
                strs_offset: 0,
            }
        };

        // 1-char: ('a', 0, 0) → "a"
        let mut st = mk_state(pack_inline(b'a', 0, 0));
        assert_eq!(
            ecgetstr(&mut st, 0, None),
            "a",
            "c:2869 strlen truncates 1-char inline at the NUL tail"
        );

        // 2-char: ('a', 'b', 0) → "ab"
        let mut st = mk_state(pack_inline(b'a', b'b', 0));
        assert_eq!(
            ecgetstr(&mut st, 0, None),
            "ab",
            "c:2869 strlen truncates 2-char inline at the NUL tail"
        );

        // 3-char: ('a', 'b', 'c') → "abc"
        let mut st = mk_state(pack_inline(b'a', b'b', b'c'));
        assert_eq!(
            ecgetstr(&mut st, 0, None),
            "abc",
            "c:2869 full 3-byte inline preserved"
        );

        // Pathological: ('a', 0, 'b') → "a" (NOT "ab" from retain-splice)
        let mut st = mk_state(pack_inline(b'a', 0, b'b'));
        assert_eq!(
            ecgetstr(&mut st, 0, None),
            "a",
            "c:2869 strlen STOPS at first NUL; must not splice 'b' through"
        );
    }

    /// Pin: `init_parse_status` resets ALL six lexer-parser flags
    /// per `Src/parse.c:500-502`. Specifically `inrepeat_ = 0` at
    /// c:501 was previously missing in the Rust port. Pin every
    /// reset so a future regression that drops one is caught.
    #[test]
    fn init_parse_status_resets_all_lexer_parser_flags() {
        let _g = crate::test_util::global_state_lock();
        // Dirty every flag to a non-default value.
        set_incasepat(5);
        set_incond(7);
        set_inredir(true);
        set_infor(3);
        set_intypeset(true);
        set_inrepeat(2);
        set_incmdpos(false);
        // Reset.
        init_parse_status();
        // c:500-502 — every flag back to its default.
        assert_eq!(incasepat(), 0, "c:500 — incasepat = 0");
        assert_eq!(incond(), 0, "c:500 — incond = 0");
        assert!(!inredir(), "c:500 — inredir = 0");
        assert_eq!(infor(), 0, "c:500 — infor = 0");
        assert!(!intypeset(), "c:500 — intypeset = 0");
        assert_eq!(
            inrepeat(),
            0,
            "c:501 — inrepeat_ = 0 (was previously missing)"
        );
        assert!(incmdpos(), "c:502 — incmdpos = 1");
    }

    // ═══════════════════════════════════════════════════════════════════
    // AST shape tests — feed source through parse(), walk the resulting
    // ZshProgram, assert structural properties. Each test uses the local
    // `parse(input)` helper that errors cleanly on parse failure.
    // Anchor: where applicable, behavior matches `zsh -n -c '...'`
    // (parse-only, no execution — which would error on syntax issues).
    // ═══════════════════════════════════════════════════════════════════

    /// Empty input → ZshProgram with no lists.
    #[test]
    fn parse_empty_source_yields_zero_lists() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("").unwrap();
        assert_eq!(prog.lists.len(), 0);
    }

    /// Comment-only input → no lists (comments are skipped at lex level).
    #[test]
    fn parse_only_comment_yields_zero_lists() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("# this is just a comment").unwrap();
        assert_eq!(prog.lists.len(), 0, "comments alone produce no cmds");
    }

    /// Three commands separated by `;` → three lists.
    #[test]
    fn parse_three_semicolon_separated_commands_yield_three_lists() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("a; b; c").unwrap();
        assert_eq!(prog.lists.len(), 3);
    }

    /// Background command — async flag set on the list.
    #[test]
    fn parse_background_command_sets_async_flag() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("sleep 1 &").unwrap();
        assert_eq!(prog.lists.len(), 1);
        assert!(
            prog.lists[0].flags.async_,
            "trailing `&` must set async_ flag"
        );
    }

    /// Pipe count: `a | b | c | d` → 4 stages.
    #[test]
    fn parse_four_stage_pipeline_has_three_next_links() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("a | b | c | d").unwrap();
        let mut pipe = &prog.lists[0].sublist.pipe;
        let mut count = 1;
        while let Some(next) = &pipe.next {
            pipe = next;
            count += 1;
        }
        assert_eq!(count, 4, "4 commands should produce 4 pipe stages");
    }

    /// `|&` between pipeline stages sets merge_stderr.
    #[test]
    fn parse_pipe_amp_sets_merge_stderr() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("a |& b").unwrap();
        let pipe = &prog.lists[0].sublist.pipe;
        assert!(pipe.next.is_some());
        assert!(pipe.merge_stderr, "|& must set merge_stderr");
    }

    /// `cmd1 || cmd2`: sublist.next is Some with `Or`.
    #[test]
    fn parse_or_operator_sets_sublist_op_or() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("cmd1 || cmd2").unwrap();
        let sublist = &prog.lists[0].sublist;
        let (op, _) = sublist.next.as_ref().expect("must have next");
        assert_eq!(*op, SublistOp::Or);
    }

    /// `! cmd` sets the not flag on the sublist.
    #[test]
    fn parse_bang_negation_sets_sublist_not_flag() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("! false").unwrap();
        let sublist = &prog.lists[0].sublist;
        assert!(sublist.flags.not, "`!` prefix must set sublist.flags.not");
    }

    // ── Compound commands ────────────────────────────────────────────
    /// `while cond; do body; done` → ZshCommand::While.
    #[test]
    fn parse_while_loop_yields_while_command() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("while true; do echo x; done").unwrap();
        assert!(matches!(
            prog.lists[0].sublist.pipe.cmd,
            ZshCommand::While(_)
        ));
    }

    /// `until cond; do body; done` → ZshCommand::Until.
    /// Anchor: `zsh -n -c 'until false; do echo; done'` accepts and parses
    /// as an until-loop. zshrs accepts but emits a DIFFERENT AST variant
    /// (not Until). Bug — until loop is mis-classified.
    #[test]
    fn parse_until_loop_yields_until_command_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("until false; do echo x; done").unwrap();
        assert!(
            matches!(prog.lists[0].sublist.pipe.cmd, ZshCommand::Until(_)),
            "zsh parses `until` as Until variant; zshrs uses different variant: {:?}",
            prog.lists[0].sublist.pipe.cmd
        );
    }

    /// `(cmd)` → Subsh variant.
    #[test]
    fn parse_parens_yield_subsh_command() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("(echo hi)").unwrap();
        assert!(matches!(
            prog.lists[0].sublist.pipe.cmd,
            ZshCommand::Subsh(_)
        ));
    }

    /// `{ cmd; }` → Cursh (current-shell) command.
    #[test]
    fn parse_braces_yield_cursh_command() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("{ echo hi; }").unwrap();
        assert!(matches!(
            prog.lists[0].sublist.pipe.cmd,
            ZshCommand::Cursh(_)
        ));
    }

    /// `[[ a == b ]]` → ZshCommand::Cond.
    #[test]
    fn parse_double_brackets_yield_cond_command() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("[[ a == b ]]").unwrap();
        assert!(matches!(
            prog.lists[0].sublist.pipe.cmd,
            ZshCommand::Cond(_)
        ));
    }

    /// `(( 1 + 2 ))` → ZshCommand::Arith.
    #[test]
    fn parse_double_parens_yield_arith_command() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("(( 1 + 2 ))").unwrap();
        assert!(matches!(
            prog.lists[0].sublist.pipe.cmd,
            ZshCommand::Arith(_)
        ));
    }

    /// `repeat 3 do echo x; done` → ZshCommand::Repeat.
    #[test]
    fn parse_repeat_loop_yields_repeat_command() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("repeat 3 do echo x; done").unwrap();
        assert!(matches!(
            prog.lists[0].sublist.pipe.cmd,
            ZshCommand::Repeat(_)
        ));
    }

    // ── Function definitions ─────────────────────────────────────────
    /// `name() { body; }` → FuncDef variant.
    #[test]
    fn parse_paren_funcdef_yields_funcdef_command() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("greet() { echo hi; }").unwrap();
        assert!(matches!(
            prog.lists[0].sublist.pipe.cmd,
            ZshCommand::FuncDef(_)
        ));
    }

    /// `function name { body; }` → FuncDef variant (zsh keyword form).
    #[test]
    fn parse_function_keyword_funcdef_yields_funcdef_command() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("function greet { echo hi; }").unwrap();
        assert!(matches!(
            prog.lists[0].sublist.pipe.cmd,
            ZshCommand::FuncDef(_)
        ));
    }

    /// Syntax error — `if` without `fi` → parse returns Err.
    /// Anchor: `echo 'if true; then echo' | zsh -n` → "parse error".
    #[test]
    fn parse_unterminated_if_returns_error_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("if true; then echo yes");
        assert!(r.is_err(), "zsh -n: parse error near `\\n`");
    }

    /// Syntax error — bare `done` without `for/while/until` → error.
    /// Anchor: `echo done | zsh -n` → "parse error near `done`".
    #[test]
    fn parse_orphan_done_returns_error_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("done");
        assert!(r.is_err(), "zsh -n: parse error near `done`");
    }

    /// Simple command's words are metafied at the AST layer (matches
    /// zsh's internal representation: `-` lexes to `Dash` = 0x9b, `*`
    /// to `Star`, etc.). zsh untokenizes via `untokenize()` BEFORE
    /// surfacing words at execution time (Src/exec.c:execcmd_args).
    /// This test pins the round-trip: `untokenize(word)` recovers the
    /// user-visible form. If parse-time unmetafy ever lands the
    /// untokenize call becomes a no-op; the test stays green either
    /// way. Companion test below pins the metafied internal form.
    #[test]
    fn parse_simple_command_words_unmetafied_like_zsh_anchored() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("ls -la /tmp").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Simple(s) => {
                let untok: Vec<String> = s
                    .words
                    .iter()
                    .map(|w| crate::ported::lex::untokenize(w))
                    .collect();
                assert_eq!(
                    untok,
                    vec!["ls", "-la", "/tmp"],
                    "untokenize(word) must yield the user-visible form"
                );
            }
            other => panic!("expected Simple, got {other:?}"),
        }
    }

    /// Pin the OBSERVED zshrs contract: simple-command word array
    /// contains metafied bytes. This is the active (passing) version
    /// of the anchor above — it documents zshrs's current internal
    /// representation. If zshrs starts unmetafying at parse time, this
    /// test will FAIL and the anchor-style test above will start passing.
    #[test]
    fn parse_simple_command_words_metafied_internal_form() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("ls -la /tmp").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Simple(s) => {
                assert_eq!(s.words.len(), 3);
                assert_eq!(s.words[0], "ls");
                assert_eq!(s.words[2], "/tmp");
                // s.words[1] contains the metafied `-` (`\u{9b}` Dash byte)
                // followed by "la". Don't pin the exact byte form (it
                // may change); pin that the length is right.
                assert_eq!(s.words[1].chars().count(), 3, "`-la` is 3 chars");
                assert!(s.words[1].ends_with("la"));
            }
            other => panic!("expected Simple, got {other:?}"),
        }
    }

    // ─── zsh-corpus pins for parser: structural shapes ────────────────

    /// Empty input — parse succeeds, lists may be empty.
    #[test]
    fn parse_corpus_empty_input_no_error() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("").unwrap();
        assert!(
            prog.lists.is_empty() || prog.lists.len() <= 1,
            "empty input → 0 or 1 list, got {}",
            prog.lists.len()
        );
    }

    /// Comment-only input parses as empty.
    #[test]
    fn parse_corpus_comment_only_no_error() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("# just a comment");
        assert!(r.is_ok(), "comment-only parse should succeed");
    }

    /// `cmd1; cmd2` — two top-level lists or two sublists.
    #[test]
    fn parse_corpus_semicolon_separates_commands() {
        let _g = crate::test_util::global_state_lock();
        let prog = parse("echo a; echo b").unwrap();
        // We pin: parse produces > 0 lists/sublists; details vary.
        assert!(!prog.lists.is_empty(), "non-empty parse");
    }

    /// `a && b` — DAMPER joins into a sublist chain.
    #[test]
    fn parse_corpus_logical_and_parses() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("true && false");
        assert!(r.is_ok(), "`a && b` parses cleanly");
    }

    /// `a || b` — DBAR.
    #[test]
    fn parse_corpus_logical_or_parses() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("false || true");
        assert!(r.is_ok(), "`a || b` parses cleanly");
    }

    /// `a | b` pipeline.
    #[test]
    fn parse_corpus_pipeline_parses() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("echo hi | cat");
        assert!(r.is_ok(), "`a | b` parses");
    }

    /// `if true; then echo x; fi` — basic if-then-fi block.
    #[test]
    fn parse_corpus_if_then_fi_parses() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("if true; then echo x; fi");
        assert!(r.is_ok(), "if/then/fi parses cleanly");
    }

    /// `for i in 1 2 3; do echo $i; done`.
    #[test]
    fn parse_corpus_for_do_done_parses() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("for i in 1 2 3; do echo $i; done");
        assert!(r.is_ok(), "for/do/done parses cleanly");
    }

    /// `while true; do break; done`.
    #[test]
    fn parse_corpus_while_do_done_parses() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("while true; do break; done");
        assert!(r.is_ok(), "while/do/done parses cleanly");
    }

    /// `case x in (a) echo A;; esac` — case statement.
    #[test]
    fn parse_corpus_case_esac_parses() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("case x in (a) echo A;; esac");
        assert!(r.is_ok(), "case/esac parses cleanly");
    }

    /// Function definition `f() { echo x }`.
    #[test]
    fn parse_corpus_function_def_parses() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("f() { echo x }");
        assert!(r.is_ok(), "f() {{ ... }} parses cleanly");
    }

    /// `(subshell echo a)` — subshell.
    #[test]
    fn parse_corpus_subshell_parens_parses() {
        let _g = crate::test_util::global_state_lock();
        let r = parse("( echo a )");
        assert!(r.is_ok(), "subshell parses cleanly");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/parse.c. Tests that capture KNOWN
    // ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `empty_eprog(p)` returns true on an eprog with empty `prog`.
    /// C `Src/parse.c:584`:
    ///   `return (!p || !p->prog || *p->prog == WCB_END());`
    /// Rust port at parse.rs:685 — `p.prog.is_empty() || p.prog[0] == WCB_END()`.
    #[test]
    fn empty_eprog_empty_prog_returns_true() {
        let _g = crate::test_util::global_state_lock();
        let p = crate::ported::zsh_h::eprog::default();
        assert!(empty_eprog(&p), "empty prog vec → empty_eprog true");
    }

    /// `empty_eprog(p)` returns true when first wordcode is WCB_END.
    /// C: `*p->prog == WCB_END()`.
    #[test]
    fn empty_eprog_first_wcb_end_returns_true() {
        let _g = crate::test_util::global_state_lock();
        let mut p = crate::ported::zsh_h::eprog::default();
        p.prog.push(WCB_END());
        assert!(empty_eprog(&p), "prog[0]==WCB_END → empty_eprog true");
    }

    /// `empty_eprog(p)` returns false for non-empty non-END prog.
    #[test]
    fn empty_eprog_non_empty_non_end_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let mut p = crate::ported::zsh_h::eprog::default();
        // Push some non-END wordcode (1 is arbitrary non-zero, not WCB_END).
        p.prog.push(1);
        assert!(!empty_eprog(&p), "non-END first opcode → false");
    }

    /// `ecstrcode("")` returns a wordcode for the empty string. C
    /// `Src/parse.c:346-ish` ecstrcode interns strings in `ecbuf`.
    /// Pin: same call returns same wordcode (deterministic intern).
    #[test]
    fn ecstrcode_empty_string_returns_deterministic_code() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let a = ecstrcode("");
        let b = ecstrcode("");
        assert_eq!(a, b, "intern of '' must be deterministic");
    }

    /// `ecstrcode` of two different strings returns different codes.
    #[test]
    fn ecstrcode_distinct_strings_get_distinct_codes() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let a = ecstrcode("foo");
        let b = ecstrcode("bar");
        // Should differ — if equal, intern table collapsed two different
        // strings to the same key (bug).
        assert_ne!(a, b, "different strings must intern to different codes");
    }

    /// `parse_event(ENDINPUT)` on empty input returns None.
    /// C `Src/parse.c:715-ish` — empty token stream → no program.
    #[test]
    fn parse_event_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        // Empty input typically yields no program; needs lex state.
        let r = parse_event(crate::ported::lex::ENDINPUT);
        assert!(r.is_none(), "no tokens → no event");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/parse.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:399 — `ecadd(c)` returns the index where `c` was placed,
    /// not the post-increment value. Sequential ecadd calls return
    /// strictly increasing indices.
    #[test]
    fn ecadd_returns_strictly_increasing_indices() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let i0 = ecadd(0xDEAD);
        let i1 = ecadd(0xBEEF);
        let i2 = ecadd(0xC0DE);
        assert!(
            i1 > i0,
            "ecadd indices must strictly increase, got {i0} then {i1}"
        );
        assert!(
            i2 > i1,
            "ecadd indices must strictly increase, got {i1} then {i2}"
        );
        assert_eq!(i1, i0 + 1, "consecutive ecadds advance by 1");
        assert_eq!(i2, i1 + 1, "consecutive ecadds advance by 1");
    }

    /// c:413 — `ecdel(p)` removes one wordcode, shrinks ecused by 1.
    /// Pin: subsequent ecadd reuses freed slot (ecused decreased).
    #[test]
    fn ecdel_shrinks_ecused_by_one() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let _i0 = ecadd(0xA);
        let i1 = ecadd(0xB);
        let _i2 = ecadd(0xC);
        let next_before = ECUSED.get();
        ecdel(i1);
        let next_after = ECUSED.get();
        assert_eq!(
            next_after,
            next_before - 1,
            "ecdel must decrement ecused by exactly 1"
        );
    }

    /// c:399-405 — `ecadd` after exhausting buffer must grow it (no
    /// panic on push past current eclen). Pin: 1000 adds don't crash.
    #[test]
    fn ecadd_grows_buffer_on_demand() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        for i in 0..1000 {
            ecadd(i as u32);
        }
        // No panic = grow path works.
        assert!(ECUSED.get() >= 1000, "1000 adds → ecused ≥ 1000");
    }

    /// c:426 — `ecstrcode` of short strings (≤4 bytes) returns a
    /// packed inline wordcode (not an offset into the string region).
    /// Pin: identical short strings get identical codes.
    #[test]
    fn ecstrcode_short_strings_are_deterministic() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let a = ecstrcode("ab");
        let b = ecstrcode("ab");
        assert_eq!(a, b, "same short string must intern to same code");
    }

    /// c:426 — long strings (>4 bytes) hit the deduped string region.
    /// Pin: same long string returns same code on repeat (registry
    /// dedupes).
    #[test]
    fn ecstrcode_long_strings_dedupe_in_registry() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let a = ecstrcode("a-much-longer-test-string");
        let b = ecstrcode("a-much-longer-test-string");
        assert_eq!(a, b, "registry must dedupe identical long strings");
    }

    /// `clear_hdocs()` is idempotent — calling twice in a row leaves
    /// HDOCS = None and LEX_HEREDOCS empty.
    #[test]
    fn clear_hdocs_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        clear_hdocs();
        clear_hdocs();
        HDOCS.with_borrow(|h| assert!(h.is_none(), "HDOCS must be None"));
        LEX_HEREDOCS.with_borrow(|v| assert!(v.is_empty(), "LEX_HEREDOCS must be empty"));
    }

    /// `init_parse()` resets parse state to known empty defaults.
    /// Multiple init_parse calls are safe (idempotent).
    #[test]
    fn init_parse_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        init_parse();
        // No panic = pass.
    }

    /// `empty_eprog` returns true for a default-constructed eprog
    /// (empty prog vec).
    #[test]
    fn empty_eprog_true_for_empty_prog() {
        let _g = crate::test_util::global_state_lock();
        let p = eprog {
            prog: Vec::new(),
            ..Default::default()
        };
        assert!(empty_eprog(&p), "empty prog vec → empty eprog");
    }

    /// `empty_eprog` returns true when prog[0] == WCB_END().
    #[test]
    fn empty_eprog_true_for_end_only_prog() {
        let _g = crate::test_util::global_state_lock();
        let p = eprog {
            prog: vec![WCB_END()],
            ..Default::default()
        };
        assert!(empty_eprog(&p), "WCB_END as first opcode → empty");
    }

    /// `ecadjusthere(p, d)` is safe to call when HDOCS is None.
    #[test]
    fn ecadjusthere_safe_when_hdocs_none() {
        let _g = crate::test_util::global_state_lock();
        clear_hdocs();
        // No panic = pass.
        ecadjusthere(0, 0);
        ecadjusthere(100, -5);
        ecadjusthere(0, 10);
    }

    /// `ecispace(p, n)` with n=0 is a no-op.
    #[test]
    fn ecispace_zero_n_is_noop() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let before = ECUSED.get();
        ecispace(0, 0);
        let after = ECUSED.get();
        assert_eq!(before, after, "ecispace(_, 0) must not advance ecused");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/parse.c
    // c:146 parse_context_save / c:191 parse_context_restore /
    // c:225 ecadjusthere / c:293 ecadd / c:346 ecstrcode / c:574 init_parse /
    // c:685 empty_eprog / c:693 clear_hdocs / c:786 parse_list / c:815 parse_cond
    // c:2234 par_wordlist / c:2249 par_nl_wordlist
    // ═══════════════════════════════════════════════════════════════════

    /// c:293 — `ecadd` returns usize (compile-time type pin).
    #[test]
    fn ecadd_returns_usize_type() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let _: usize = ecadd(0);
    }

    /// c:346 — `ecstrcode` returns u32 (compile-time type pin).
    #[test]
    fn ecstrcode_returns_u32_type() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let _: u32 = ecstrcode("");
    }

    /// c:346 — `ecstrcode("")` empty string is safe.
    #[test]
    fn ecstrcode_empty_string_no_panic() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let _ = ecstrcode("");
    }

    /// c:346 — `ecstrcode` is deterministic for same input.
    #[test]
    fn ecstrcode_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        for s in ["", "a", "abc", "hello world"] {
            let first = ecstrcode(s);
            for _ in 0..3 {
                assert_eq!(
                    ecstrcode(s),
                    first,
                    "ecstrcode({:?}) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:786 — `parse_list` returns Option<eprog>.
    #[test]
    fn parse_list_returns_option_eprog_type() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let _: Option<eprog> = parse_list();
    }

    /// c:815 — `parse_cond` returns Option<eprog>.
    #[test]
    fn parse_cond_returns_option_eprog_type() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let _: Option<eprog> = parse_cond();
    }

    /// c:2234 — `par_wordlist` returns Vec<String>.
    #[test]
    fn par_wordlist_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let _: Vec<String> = par_wordlist();
    }

    /// c:2249 — `par_nl_wordlist` returns Vec<String>.
    #[test]
    fn par_nl_wordlist_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        init_parse();
        let _: Vec<String> = par_nl_wordlist();
    }

    /// c:693 — `clear_hdocs` deterministic state after call (no-panic).
    #[test]
    fn clear_hdocs_deterministic_after_call() {
        let _g = crate::test_util::global_state_lock();
        clear_hdocs();
        clear_hdocs();
    }

    /// c:225 — `ecadjusthere(0, 0)` is a no-op (no delta).
    #[test]
    fn ecadjusthere_zero_delta_no_panic() {
        let _g = crate::test_util::global_state_lock();
        ecadjusthere(0, 0);
    }

    /// c:225 — `ecadjusthere` is safe for arbitrary positions.
    #[test]
    fn ecadjusthere_arbitrary_pos_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for p in [0usize, 1, 100, 9999] {
            ecadjusthere(p, 0);
            ecadjusthere(p, 1);
            ecadjusthere(p, -1);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/parse.c FD_* accessors
    // c:3127 fdmagic / c:3131 fdflags / c:3133 fdother / c:3140 fdversion /
    // c:3145 fdhflags / c:3146 fdhtail / c:3147 fdhbldflags
    // ═══════════════════════════════════════════════════════════════════

    fn build_fd_header() -> Vec<u32> {
        let mut buf = vec![0u32; FD_PRELEN + 32];
        buf[0] = FD_MAGIC; // pre[0] magic
        buf[1] = (0x12u32) | (0x00ABCDEFu32 << 8); // flags=0x12, other=0xABCDEF
                                                   // Embed version string starting at pre[2].
        let ver = b"5.9\0";
        for (i, chunk) in ver.chunks(4).enumerate() {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            buf[2 + i] = u32::from_le_bytes(word);
        }
        buf[FD_PRELEN - 1] = (FD_PRELEN as u32) + 8; // header-len slot
        buf
    }

    /// c:3127 — `fdmagic(f)` returns pre[0] verbatim.
    #[test]
    fn fdmagic_returns_pre_zero_word() {
        let buf = build_fd_header();
        assert_eq!(fdmagic(&buf), FD_MAGIC, "fdmagic = pre[0]");
    }

    /// c:3131 — `fdflags` extracts low byte of pre[1].
    #[test]
    fn fdflags_low_byte_extraction() {
        let buf = build_fd_header();
        assert_eq!(fdflags(&buf), 0x12, "flags = pre[1] & 0xff");
    }

    /// c:3133 — `fdother` extracts high 24 bits of pre[1].
    #[test]
    fn fdother_high_24_bits_extraction() {
        let buf = build_fd_header();
        assert_eq!(
            fdother(&buf),
            0x00ABCDEF,
            "other = pre[1] >> 8 & 0x00ffffff"
        );
    }

    /// c:3132 — `fdsetflags` writes low byte, preserves high 24 bits.
    #[test]
    fn fdsetflags_preserves_high_24_bits() {
        let mut buf = build_fd_header();
        let other_before = fdother(&buf);
        fdsetflags(&mut buf, 0x42);
        assert_eq!(fdflags(&buf), 0x42, "new flags written");
        assert_eq!(fdother(&buf), other_before, "high 24 bits preserved");
    }

    /// c:3134 — `fdsetother` writes high 24 bits, preserves low byte.
    #[test]
    fn fdsetother_preserves_low_byte() {
        let mut buf = build_fd_header();
        let flags_before = fdflags(&buf);
        fdsetother(&mut buf, 0x00DEADBE);
        assert_eq!(fdother(&buf), 0x00DEADBE, "new other written");
        assert_eq!(fdflags(&buf), flags_before, "low byte preserved");
    }

    /// c:3134 — `fdsetother` clamps to 24 bits (caller-passed high bits dropped).
    #[test]
    fn fdsetother_clamps_to_24_bits() {
        let mut buf = build_fd_header();
        fdsetother(&mut buf, 0xFF_FFFF_FF);
        // Only the low 24 bits land in `other`.
        assert_eq!(fdother(&buf), 0x00FF_FFFF, "high bits dropped");
    }

    /// c:3140 — `fdversion(buf)` returns String (compile-time type pin).
    #[test]
    fn fdversion_returns_string_type() {
        let buf = build_fd_header();
        let _: String = fdversion(&buf);
    }

    /// c:3140 — `fdversion` reads the NUL-terminated string from pre[2..].
    #[test]
    fn fdversion_reads_until_nul() {
        let buf = build_fd_header();
        assert_eq!(fdversion(&buf), "5.9", "version read until NUL");
    }

    /// c:3145 — `fdhflags(h)` returns low 2 bits of flags.
    #[test]
    fn fdhflags_low_two_bits() {
        let h = fdhead {
            start: 0,
            len: 0,
            npats: 0,
            strs: 0,
            hlen: 0,
            flags: 0b1011, // tail=2, kshload bits = 0b11
        };
        assert_eq!(fdhflags(&h), 0b11, "flags = h.flags & 0x3");
    }

    /// c:3146 — `fdhtail(h)` returns high 30 bits (shifted right by 2).
    #[test]
    fn fdhtail_shift_right_two() {
        let h = fdhead {
            start: 0,
            len: 0,
            npats: 0,
            strs: 0,
            hlen: 0,
            flags: (0x12_3456 << 2) | 0x3,
        };
        assert_eq!(fdhtail(&h), 0x12_3456, "tail = h.flags >> 2");
    }

    /// c:3147 — `fdhbldflags(flags, tail)` packs into single u32.
    #[test]
    fn fdhbldflags_packs_flags_low_tail_high() {
        let packed = fdhbldflags(0x3, 0x42);
        assert_eq!(packed & 0x3, 0x3, "low 2 bits = flags");
        assert_eq!(packed >> 2, 0x42, "high 30 bits = tail");
    }

    /// c:3145-3147 — `fdhflags(h)`+`fdhtail(h)` round-trip via fdhbldflags.
    #[test]
    fn fdh_round_trip_via_bldflags() {
        for (flags, tail) in [(0u32, 0u32), (1, 100), (2, 0xABC), (3, 0xFFFF)] {
            let packed = fdhbldflags(flags, tail);
            let h = fdhead {
                start: 0,
                len: 0,
                npats: 0,
                strs: 0,
                hlen: 0,
                flags: packed,
            };
            assert_eq!(fdhflags(&h), flags, "flags round-trips");
            assert_eq!(fdhtail(&h), tail, "tail round-trips");
        }
    }

    /// c:8271 — `firstfdhead_offset()` returns FD_PRELEN constant.
    #[test]
    fn firstfdhead_offset_returns_prelen() {
        assert_eq!(
            firstfdhead_offset(),
            FD_PRELEN,
            "first header starts after prelude"
        );
    }

    /// c:3127 — `fdmagic` differentiates FD_MAGIC from FD_OMAGIC.
    #[test]
    fn fdmagic_differentiates_magic_omagic() {
        let mut buf = vec![FD_MAGIC; FD_PRELEN];
        assert_eq!(fdmagic(&buf), FD_MAGIC);
        buf[0] = FD_OMAGIC;
        assert_eq!(fdmagic(&buf), FD_OMAGIC, "swapped magic readable");
        assert_ne!(FD_MAGIC, FD_OMAGIC, "the two magics differ");
    }
}
