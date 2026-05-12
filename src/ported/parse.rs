//! Zsh parser — direct port from zsh/Src/parse.c.
//!
//! Pulls tokens via the lex.rs free fns (zshlex/tok/tokstr) and
//! builds an AST tree (relocated to src/extensions/zsh_ast.rs as a
//! Rust-only IR) plus emits wordcode into ECBUF via the P9b/P9c
//! pipeline. Follows the zsh grammar closely; productions match
//! `par_*` in Src/parse.c.

use super::lex::{
    lextok, AMPER, AMPERBANG, AMPOUTANG, BANG_TOK, BARAMP, BAR_TOK, CASE, COPROC, DAMPER, DBAR,
    DINANG, DINANGDASH, DINBRACK, DINPAR, DOLOOP, DONE, DOUTANG, DOUTANGAMP, DOUTANGAMPBANG,
    DOUTANGBANG, DOUTBRACK, DOUTPAR, DSEMI, ELIF, ELSE, ENDINPUT, ENVARRAY, ENVSTRING, ESAC, FI,
    FOR, FOREACH, FUNC, IF, INANGAMP, INANG_TOK, INBRACE_TOK, INOUTANG, INOUTPAR, INPAR_TOK,
    IS_REDIROP, LEXERR, NEWLIN, NOCORRECT, NULLTOK, OUTANGAMP, OUTANGAMPBANG, OUTANGBANG,
    OUTANG_TOK, OUTBRACE_TOK, OUTPAR_TOK, REPEAT, SELECT, SEMI, SEMIAMP, SEMIBAR, SEPER,
    STRING_LEX, THEN, TIME, TRINANG, TYPESET, UNTIL, WHILE, ZEND,
};
use super::zsh_h::{
    redir, wc_code, wordcode, DASH, EQUALS, IS_DASH, REDIRF_FROM_HEREDOC, REDIR_APP, REDIR_APPNOW,
    REDIR_ERRAPP, REDIR_ERRAPPNOW, REDIR_ERRWRITE, REDIR_ERRWRITENOW, REDIR_HEREDOC,
    REDIR_HEREDOCDASH, REDIR_HERESTR, REDIR_INPIPE, REDIR_MERGEIN, REDIR_MERGEOUT, REDIR_OUTPIPE,
    REDIR_READ, REDIR_READWRITE, REDIR_WRITE, REDIR_WRITENOW, WC_REDIR, WC_REDIR_FROM_HEREDOC,
    WC_REDIR_TYPE, WC_REDIR_VARID,
};
use crate::zsh_h::{BANG, COND_AND, COND_NOT, COND_STRGTR, COND_STRLT, EC_DUP, EC_NODUP, INANG, INPAR, OUTANG, OUTPAR, WCB_COND};
use crate::ported::lex::set_tok;
use crate::ported::utils::{zerr, zwarnnam};
use crate::ported::zsh_h::{
    eprog, COND_MOD, COND_MODI, COND_NT, COND_REGEX, COND_STRDEQ, COND_STREQ,
    COND_STRNEQ, EF_HEAP, EF_REAL, OPT_ISSET, PM_UNDEFINED, WC_SUBLIST_COPROC,
    WC_SUBLIST_NOT,
};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use serde::{Deserialize, Serialize};

// Wordcode-buffer thread-locals — direct port of `Src/parse.c:269-285`
// file-statics. Per-evaluator (bucket-1 in PORT_PLAN.md): each worker
// thread parsing a separate program needs its own wordcode buffer.
//
// ECBUF: the wordcode array being built. C `Wordcode ecbuf`
// (parse.c:275).
// ECLEN: allocated entries in ECBUF (parse.c:269).
// ECUSED: entries actually used so far (parse.c:271).
// ECNPATS: count of patterns referenced by ECBUF (parse.c:273).
// ECSOFFS / ECSSUB: byte offsets into the deferred string region
// (parse.c:279). ECSSUB subtracts substring overlap.
// ECNFUNC: count of functions defined so far (parse.c:285).
// ECSTRS_INDEX: dedup index for long strings — C uses a binary tree
// of `struct eccstr` (zsh.h:836); the canonical Eccstr port exists
// at zsh_h::eccstr but stays unused at runtime here. The HashMap
// preserves the API contract (lookup by (nfunc, str) → offs) with
// simpler ownership semantics.
thread_local! {
    pub static ECBUF: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(Vec::new());
    static ECLEN: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECUSED: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECNPATS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECSOFFS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECSSUB: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECNFUNC: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static ECSTRS_INDEX: std::cell::RefCell<std::collections::HashMap<(i32, String), u32>>
        = std::cell::RefCell::new(std::collections::HashMap::new());
    /// Reverse index for `ecgetstr`: offs → owned string. Populated
    /// at ecstrcode time so the consumer can recover the string from
    /// the wordcode offs without walking the encode-time HashMap.
    pub static ECSTRS_REVERSE: std::cell::RefCell<std::collections::HashMap<u32, String>>
        = std::cell::RefCell::new(std::collections::HashMap::new());
}

// Direct port of `Src/parse.c:287-289` grow-policy constants.
const EC_INIT_SIZE: i32 = 256;
const EC_DOUBLE_THRESHOLD: i32 = 32768;
const EC_INCREMENT: i32 = 1024;

// Parser recursion + iteration safety counters as file-scope
// thread_locals (Rust-only — no C analog; C uses OS stack overflow).
thread_local! {
    pub static PARSER_RECURSION_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub static PARSER_GLOBAL_ITERATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// =============================================================================
// Wordcode read helpers — used by text.rs's `gettext2` and exec dispatch
// to walk a compiled Eprog without re-running the parser. These are the
// only `Src/parse.c` functions ported so far in this file; the recursive-
// descent parser (par_event / par_list / par_cmd / par_*) follows
// below as free fns at module scope.
// =============================================================================

/// Port of `ecgetstr(Estate s, int dup, int *tokflag)` from `Src/parse.c:2855`.
///
/// `strs` must be the **current** string pool tail (`s->strs` in C); it advances
/// separately via `estate.strs_offset` in `text.c` callers.
/// WARNING: param names don't match C — Rust=(prog, strs, pc, dup, tokflag) vs C=(s, dup, tokflag)
pub fn ecgetstr(
    prog: &[u32],
    strs: &[u8],
    pc: &mut usize,
    dup: i32,
    tokflag: Option<&mut i32>,
) -> String {
    if *pc >= prog.len() {
        return String::new();
    }
    let c = prog[*pc];
    *pc += 1;
    if let Some(tf) = tokflag {
        *tf = i32::from((c & 1) != 0);
    }
    if c == 6 || c == 7 {
        return String::new();
    }
    let r: String = if (c & 2) != 0 {
        let b0 = ((c >> 3) & 0xff) as u8;
        let b1 = ((c >> 11) & 0xff) as u8;
        let b2 = ((c >> 19) & 0xff) as u8;
        let mut v = vec![b0, b1, b2];
        v.retain(|&x| x != 0);
        String::from_utf8_lossy(&v).into_owned()
    } else {
        let off = (c >> 2) as usize;
        if off >= strs.len() {
            String::new()
        } else {
            let tail = &strs[off..];
            let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
            String::from_utf8_lossy(&tail[..end]).into_owned()
        }
    };
    let _ = EC_NODUP;
    let _need_dup = dup == EC_DUP || (dup != EC_NODUP && (c & 1) != 0);
    r
}

/// Port of `ecgetredirs(Estate s)` from `Src/parse.c:2959`.
///
/// `strs` must be the same tail `ecgetstr` uses (`s->strs` / `estate.strs` from offset).
/// WARNING: param names don't match C — Rust=(prog, strs, pc) vs C=(s)
pub fn ecgetredirs(prog: &[wordcode], strs: &[u8], pc: &mut usize) -> Vec<redir> {
    let mut ret: Vec<redir> = Vec::new(); // c:2959 `LinkList ret = newlinklist();`
    if *pc >= prog.len() {
        return ret;
    }
    let mut code = prog[*pc]; // c:2962 `wordcode code = *s->pc++;`
    *pc += 1;

    loop {
        if wc_code(code) != WC_REDIR {
            // c:2988-2989 `s->pc--` then break from while
            *pc = (*pc).saturating_sub(1);
            break;
        }

        let typ = WC_REDIR_TYPE(code); // c:2967 `r->type = WC_REDIR_TYPE(code);`
        if *pc >= prog.len() {
            break;
        }
        let fd1_w = prog[*pc]; // c:2968 `r->fd1 = *s->pc++;`
        *pc += 1;

        let name = ecgetstr(prog, strs, pc, EC_DUP, None); // c:2969 `r->name = ecgetstr(...)`

        let (flags, here_terminator, munged_here_terminator) = if WC_REDIR_FROM_HEREDOC(code) != 0 {
            // c:2970-2973
            let term = ecgetstr(prog, strs, pc, EC_DUP, None);
            let munged = ecgetstr(prog, strs, pc, EC_DUP, None);
            (REDIRF_FROM_HEREDOC, Some(term), Some(munged))
        } else {
            // c:2974-2977
            (0, None, None)
        };

        let varid = if WC_REDIR_VARID(code) != 0 {
            // c:2979-2980
            Some(ecgetstr(prog, strs, pc, EC_DUP, None))
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

        if *pc >= prog.len() {
            break;
        }
        code = prog[*pc]; // c:2986 `code = *s->pc++;`
        *pc += 1;
    }

    ret // c:2990 `return ret`
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
// P9d rewrite (par_* emitting wordcode + exec.rs reading wordcode)
// retires them entirely — until then, callers reach them via this
// re-export.
pub use crate::extensions::heredoc_ast::HereDoc;
pub use crate::extensions::zsh_ast::{
    CaseArm, CaseTerm, CaseTerminator, CompoundCommand, ForList, HereDocInfo, ListFlags, ListOp,
    Redirect, RedirectOp, ShellCommand, ShellWord, SimpleCommand, SublistFlags, SublistOp,
    VarModifier, ZshAssign, ZshAssignValue, ZshCase, ZshCommand, ZshCond, ZshFor, ZshFuncDef,
    ZshIf, ZshList, ZshParamFlag, ZshPipe, ZshProgram, ZshRedir, ZshRepeat, ZshSimple, ZshSublist,
    ZshTry, ZshWhile,
};
use crate::lex::{set_incmdpos, set_incond, tok, tokstr, zshlex};
// === end AST relocation ===

// Parser state lives in file-scope thread_locals:
//   - LEX_* (lexer side, matching Src/lex.c file-statics)
//   - ECBUF / ECLEN / ECUSED / ECNPATS / ECSOFFS / ECSSUB / ECNFUNC /
//     ECSTRS_INDEX / ECSTRS_REVERSE (wordcode-emission state, matching
//     Src/parse.c file-statics)
//   - PARSER_RECURSION_DEPTH / PARSER_GLOBAL_ITERATIONS (Rust-only
//     safety counters; no C analog — C relies on OS stack overflow).
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
/// shapes (Vec<HereDoc>, stubbed wordcode fields) suited to zshrs's
/// pre-wordcode AST architecture; the consolidation happens in P9b.
#[allow(non_camel_case_types)]
#[derive(Debug, Default, Clone)]
pub struct parse_stack {
    // ── Direct port of struct parse_stack at zsh.h:3099-3109 ──
    /// Pending heredocs awaiting body collection. C: `struct heredocs
    /// *hdocs` (zsh.h:3100). zshrs uses Vec<HereDoc> until Phase 9b
    /// (PORT_PLAN.md) reinstates C's linked-list shape.
    pub hdocs: Vec<HereDoc>,
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
    // P8: Rust-only safety counters (recursion_depth, global_iterations)
    // migrated to PARSER_RECURSION_DEPTH + PARSER_GLOBAL_ITERATIONS
    // thread_locals. parse_stack no longer carries them — matches C
    // exactly (C's struct parse_stack has no analog).
}

// Old uppercase Rust-only `ParseStack` is gone. Compat alias so
// existing call sites (context.rs) keep resolving until the
// rename ripples through.
#[allow(non_camel_case_types)]
pub type ParseStack = parse_stack;

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
                resolve_redir(r, bodies);
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
                resolve_redir(r, bodies);
            }
            fill_in_command(inner, bodies);
        }
        ZshCommand::Time(None) | ZshCommand::Cond(_) | ZshCommand::Arith(_) => {}
    }
}

fn resolve_redir(r: &mut ZshRedir, bodies: &[HereDocInfo]) {
    if let Some(idx) = r.heredoc_idx {
        if let Some(info) = bodies.get(idx) {
            r.heredoc = Some(info.clone());
        }
    }
}

/// If `list` is a Simple containing one word that ends in the
/// `<INPAR><OUTPAR>` token pair (the lexer-port encoding of `()`),
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
    let suffix = "\u{88}\u{8a}"; // INPAR + OUTPAR
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
    // P8: reset Rust-only safety counters at parser construction.
    PARSER_GLOBAL_ITERATIONS.set(0);
    PARSER_RECURSION_DEPTH.set(0);
    crate::ported::lex::lex_init(input);
}

/// Check iteration limit; returns true if exceeded
#[inline]
fn check_limit() -> bool {
    PARSER_GLOBAL_ITERATIONS.set(PARSER_GLOBAL_ITERATIONS.get() + 1);
    PARSER_GLOBAL_ITERATIONS.get() > 10_000
}

/// Check recursion depth; returns true if exceeded
#[inline]
fn check_recursion() -> bool {
    PARSER_RECURSION_DEPTH.get() > MAX_RECURSION_DEPTH
}

/// Direct port of `parse_context_save(struct parse_stack *ps, int toplevel)` at `Src/parse.c:295`.
/// Snapshots the lexer-side file-statics (which currently live on
/// `lexer` until Phase 7 dissolution makes them file-scope
/// thread_local!s) plus the pending heredoc list, plus the
/// wordcode-buffer state (STUB until Phase 9b). Saves Rust-only
/// recursion counters too so nested parses get fresh limits.
/// WARNING: param names don't match C — Rust=(ps) vs C=(ps, toplevel)
pub fn parse_context_save(ps: &mut parse_stack) {
    // parse.c:299 — `ps->hdocs = hdocs; hdocs = NULL;`
    ps.hdocs = crate::ported::lex::heredocs_take();
    // parse.c:302-310 — save lexer-side state.
    ps.incmdpos = crate::ported::lex::incmdpos();
    // parse.c:303 — aliasspaceflag — not yet a LEX_* thread_local.
    // STUB; Phase 7 wires it. Same for the few below marked STUB.
    ps.aliasspaceflag = 0;
    ps.incond = crate::ported::lex::incond();
    ps.inredir = crate::ported::lex::inredir();
    ps.incasepat = crate::ported::lex::incasepat();
    ps.isnewlin = crate::ported::lex::isnewlin();
    ps.infor = crate::ported::lex::infor();
    ps.inrepeat_ = crate::ported::lex::inrepeat();
    ps.intypeset = crate::ported::lex::intypeset();
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
    // P8: counters are file-scope thread_locals; reset them on save
    // (matches the C parse_context_save clear-buffer semantics).
    // Nested parses get a fresh limit; outer parse's count is lost
    // — acceptable since the counters are safety nets, not state.
    PARSER_RECURSION_DEPTH.set(0);
    PARSER_GLOBAL_ITERATIONS.set(0);
    crate::ported::lex::set_incmdpos(true);
    crate::ported::lex::set_incond(0);
    crate::ported::lex::set_inredir(false);
    crate::ported::lex::set_incasepat(0);
    crate::ported::lex::set_infor(0);
    crate::ported::lex::set_inrepeat(0);
    crate::ported::lex::set_intypeset(false);
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
    crate::ported::lex::heredocs_set(ps.hdocs.clone());
    crate::ported::lex::set_incmdpos(ps.incmdpos);
    // aliasspaceflag STUB until Phase 7.
    crate::ported::lex::set_incond(ps.incond);
    crate::ported::lex::set_inredir(ps.inredir);
    crate::ported::lex::set_incasepat(ps.incasepat);
    crate::ported::lex::set_isnewlin(ps.isnewlin);
    crate::ported::lex::set_infor(ps.infor);
    crate::ported::lex::set_inrepeat(ps.inrepeat_);
    crate::ported::lex::set_intypeset(ps.intypeset);
    // ecbuf/eclen/ecused/ecnpats/ecstrs/ecsoffs/ecssub/ecnfunc
    // STUB until Phase 9b.
    // P8: counters not restored — see parse_context_save comment.

    // parse.c:354 — `errflag &= ~ERRFLAG_ERROR;` — clear the
    // error flag so the outer parse sees a clean state.
    crate::ported::utils::errflag.fetch_and(
        !crate::ported::utils::ERRFLAG_ERROR,
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Initialize parser status. Direct port of zsh/Src/parse.c:491
/// `init_parse_status`. Clears the per-parse-call lexer flags
/// so a fresh parse starts from cmd-position with no nesting
/// state inherited from a prior parse.
pub fn init_parse_status() {
    // parse.c:500-502 — `incasepat = incond = inredir = infor =
    // intypeset = 0; inrepeat_ = 0; incmdpos = 1;`
    crate::ported::lex::set_incasepat(0);
    crate::ported::lex::set_incond(0);
    crate::ported::lex::set_inredir(false);
    crate::ported::lex::set_infor(0);
    crate::ported::lex::set_intypeset(false);
    crate::ported::lex::set_incmdpos(true);
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

    PARSER_RECURSION_DEPTH.set(0);
    PARSER_GLOBAL_ITERATIONS.set(0);
    // parse.c:522 — `init_parse_status();`
    init_parse_status();
}

/// Check whether the parsed program is empty. Direct port of
/// zsh/Src/parse.c:584 `empty_eprog`. C version checks
/// `*p->prog == WCB_END()` (single end-of-wordcode marker).
/// zshrs version checks the AST node count.
pub fn empty_eprog(prog: &ZshProgram) -> bool {
    prog.lists.is_empty()
}

/// Clear pending here-document list. Direct port of
/// zsh/Src/parse.c:591 `clear_hdocs`. The C version walks
/// the global `hdocs` linked list and frees each node. zshrs
/// stores pending heredocs on the lexer's `heredocs` Vec —
/// truncating it has the same effect.
pub fn clear_hdocs() {
    crate::ported::lex::heredocs_clear();
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
    crate::ported::lex::set_tok(ENDINPUT);
    crate::ported::lex::set_incmdpos(true);
    crate::ported::lex::zshlex();
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
    while crate::ported::lex::tok() == SEPER {
        // parse.c:640-641 — at top-level (endtok == ENDINPUT),
        // a SEPER on a fresh line ends the event.
        if crate::ported::lex::isnewlin() > 0 && endtok == ENDINPUT {
            return false;
        }
        crate::ported::lex::zshlex();
    }
    // parse.c:644-647 — terminate on EOF or matching close-token.
    if crate::ported::lex::tok() == ENDINPUT {
        return false;
    }
    if crate::ported::lex::tok() == endtok {
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

/// Wire a here-document body onto the redirection token that
/// requested it. Direct port of zsh/Src/parse.c:2347
/// `setheredoc`. Called when a heredoc terminator has been
/// matched and the body is ready to be attached to the redir.
///
/// zshrs port note: zsh's setheredoc patches the wordcode
/// in-place via `pc[1] = ecstrcode(doc); pc[2] = ecstrcode(term);`.
/// zshrs threads heredoc bodies through `HereDocInfo` structs
/// that resolve_redir applies during the post-parse fill_in pass.
/// This method is the AST-side equivalent: writes back to the
/// matching redir node by index.
pub fn setheredoc(_pc: usize, _redir_type: i32, _doc: &str, _term: &str, _munged_term: &str) {
    // zshrs's heredoc resolution happens in fill_in_command /
    // resolve_redir at parse top. This stub exists for API
    // parity with the C signature; live wiring happens via
    // heredocs which the post-parse pass consumes.
}

/// Parse a wordlist for `for ... in WORDS;`. Direct port of
/// zsh/Src/parse.c:2362 `par_wordlist`. Reads STRING tokens
/// until the next SEPER / SEMI / NEWLIN.
pub fn par_wordlist() -> Vec<String> {
    let mut out = Vec::new();
    // parse.c:2362-2378 — collect STRINGs into the wordlist.
    while crate::ported::lex::tok() == STRING_LEX {
        if let Some(text) = crate::ported::lex::tokstr() {
            out.push(text);
        }
        crate::ported::lex::zshlex();
    }
    out
}

/// Parse a newline-separated wordlist. Direct port of
/// zsh/Src/parse.c:2379 `par_nl_wordlist`. Like
/// par_wordlist but tolerates leading/trailing newlines.
pub fn par_nl_wordlist() -> Vec<String> {
    // parse.c:2380-2381 — skip leading newlines.
    while crate::ported::lex::tok() == NEWLIN {
        crate::ported::lex::zshlex();
    }
    let out = par_wordlist();
    // parse.c:2395-2397 — skip trailing newlines.
    while crate::ported::lex::tok() == NEWLIN {
        crate::ported::lex::zshlex();
    }
    out
}

/// Read an integer from the next cond token. NOT a direct C port —
/// the C `get_cond_num(char *tst)` (parse.c:2643) is the
/// string-lookup helper ported below. This Rust-only helper exists
/// to support the AST cond-walker (`par_cond_*` analogs) when it
/// needs a numeric literal from the current lex position.
pub fn read_cond_num() -> Option<i64> {
    if crate::ported::lex::tok() != STRING_LEX {
        return None;
    }
    let text = crate::ported::lex::tokstr()?;
    let parsed = text.parse::<i64>().ok()?;
    crate::ported::lex::zshlex();
    Some(parsed)
}

/// Port of `get_cond_num(char *tst)` from `Src/parse.c:2643`. Returns
/// the index of `tst` in `{"nt","ot","ef","eq","ne","lt","gt","le","ge"}`
/// or `-1` if not a recognized binary cond operator.
pub fn get_cond_num(tst: &str) -> i32 {                                       // c:2643
    const CONDSTRS: [&str; 9] = [
        "nt", "ot", "ef", "eq", "ne", "lt", "gt", "le", "ge",                 // c:2647
    ];
    for (i, &c) in CONDSTRS.iter().enumerate() {
        if c == tst {
            return i as i32;                                                  // c:2654
        }
    }
    -1                                                                         // c:2656
}

/// Emit a parser-level error. Direct port of zsh/Src/parse.c
/// 2733-2766 `yyerror`. C version fills a per-event error buffer
/// and sets errflag. zshrs pushes onto errors which the
/// caller drains via parse()'s Result return.
pub fn yyerror(msg: &str) {
    // parse.c:2735-2765 — zsh's yyerror collects the offending
    // token's literal text + line number. zshrs already does
    // this via error() with the lexer's toklineno.
    error(msg);
}

// ============================================================
// Wordcode emission stubs (parse.c private helpers)
//
// The following functions are direct counterparts of zsh's
// private wordcode-emission helpers in parse.c. zsh uses these
// to write u32 opcodes into a flat `ecbuf` array; zshrs builds
// an AST tree and never emits wordcode at the parse layer.
// The implementations are documented stubs that preserve the
// function signatures + cite the C source. Real wordcode would
// be emitted later by compile_zsh.rs walking the AST.
//
// Listed for port-surface completeness so every parse.c symbol
// has a Rust counterpart even when the algorithm is moot in the
// AST architecture.
// ============================================================

/// Patch a list-placeholder wordcode with its actual opcode +
/// jump distance. Direct port of zsh/Src/parse.c:738
/// `set_list_code`. zsh emits an `ecadd(0)` placeholder before
/// par_sublist runs, then comes back through set_list_code to
/// rewrite the slot with WCB_LIST(type, distance) once the
/// sublist's final length is known.
///
/// zshrs port note: zshrs builds AST nodes inline so there's
/// no placeholder to patch. The ZshList { sublist, flags }
/// node is created with the right flags from the start.
/// Stub provided for port-surface completeness.
pub fn set_list_code(_p: usize, _type_code: i32, _cmplx: bool) {
    // parse.c:740-748 — wordcode patching. zshrs no-op.
}

/// Patch a sublist-placeholder wordcode with its actual opcode.
/// Direct port of zsh/Src/parse.c:755 `set_sublist_code`.
/// Same role as set_list_code at the sublist level.
pub fn set_sublist_code(p: usize, type_code: i32, flags: i32, skip: i32, cmplx: bool) {
    // parse.c:757-762 — patch the wordcode at p. zshrs P9b: write
    // into ECBUF directly so par_* productions can build sublist
    // wordcode with deferred type/skip patching.
    let _ = cmplx; // cmplx encoded into flags by caller; not
                   // re-encoded here.
    ECBUF.with_borrow_mut(|buf| {
        if p < buf.len() {
            let data = (type_code | (flags << 1) | (skip << 6)) as u32;
            buf[p] = crate::ported::zsh_h::WC_SUBLIST | (data << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
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
pub fn ecstrcode(s: &str) -> u32 {
    // parse.c:432-470 — short-string inline-pack vs registry-offset.
    let l = s.len() + 1; // include NUL terminator (matches C strlen+1)
    let t = crate::ported::utils::has_token(s);
    let bytes = s.as_bytes();
    if l <= 4 {
        // parse.c:436-445 — short-string inline pack.
        let mut c: u32 = if t { 3 } else { 2 };
        match l {
            4 => {
                c |= (bytes[2] as u32) << 19;
                c |= (bytes[1] as u32) << 11;
                c |= (bytes[0] as u32) << 3;
            }
            3 => {
                c |= (bytes[1] as u32) << 11;
                c |= (bytes[0] as u32) << 3;
            }
            2 => {
                c |= (bytes[0] as u32) << 3;
            }
            1 => {
                // parse.c:443 — empty string special case.
                c = if t { 7 } else { 6 };
            }
            _ => {}
        }
        c
    } else {
        // parse.c:448-470 — long string: dedup by (nfunc, hashval,
        // str) and return existing offs if found, else allocate
        // a new offs into the string region. zshrs uses HashMap
        // for the dedup index — the canonical eccstr binary tree
        // (zsh.h:836) is defined but not used at runtime here;
        // the API contract (return offs for a given string) is
        // preserved.
        let key = (ECNFUNC.get(), s.to_string());
        if let Some(&offs) = ECSTRS_INDEX.with_borrow(|m| m.get(&key).copied()).as_ref() {
            return offs;
        }
        let offs = (((ECSOFFS.get() - ECSSUB.get()) as u32) << 2) | if t { 1 } else { 0 };
        ECSTRS_INDEX.with_borrow_mut(|m| {
            m.insert(key, offs);
        });
        ECSTRS_REVERSE.with_borrow_mut(|m| {
            m.insert(offs, s.to_string());
        });
        ECSOFFS.set(ECSOFFS.get() + l as i32);
        offs
    }
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
        // parse.c:2872-2873 — long string via offs lookup.
    let s = ECSTRS_REVERSE
        .with_borrow(|m| m.get(&c).cloned())
        .unwrap_or_default();
    (s, next)
}

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
/// `dupeprog`. C version deep-copies the wordcode array + string
/// table + pattern progs. zshrs uses Clone on the AST.
pub fn dupeprog(prog: &ZshProgram) -> ZshProgram {
    prog.clone()
}

/// Increment an Eprog's reference count. Direct port of
/// zsh/Src/parse.c:2813 `useeprog`. zshrs no-op (Rust
/// ownership).
pub fn useeprog(_prog: &ZshProgram) {
    // parse.c:2815-2821 — `prog->nref++` if not heap-allocated.
    // zshrs no-op.
}

/// Decrement / free an Eprog. Direct port of
/// zsh/Src/parse.c:2823 `freeeprog`. zshrs no-op (drop on
/// scope-exit).
pub fn freeeprog(_prog: ZshProgram) {
    // parse.c:2825-2853 — decrement nref, free if zero. zshrs
    // drops via Rust ownership.
}

// ============================================================
// Wordcode runtime getters (parse.c:2853-3060)
//
// These read packed wordcode out of a running Eprog at execution
// time. zshrs's executor walks the AST directly so these are
// stubs that preserve the C signatures + cite the source.
// ============================================================

/// Read a packed string without consuming the wordcode pointer.
/// Direct port of zsh/Src/parse.c:2891 `ecrawstr`. zshrs
/// no-op.
pub fn ecrawstr() -> String {
    String::new()
}

/// Read a NUL-terminated string array from wordcode. Direct port
/// of zsh/Src/parse.c:2917 `ecgetarr`. zshrs no-op.
pub fn ecgetarr(_num: usize, _dup: bool) -> Vec<String> {
    Vec::new()
}

/// Read a linked-list of strings from wordcode. Direct port of
/// zsh/Src/parse.c:2937 `ecgetlist`. zshrs no-op.
pub fn ecgetlist(_num: usize, _dup: bool) -> Vec<String> {
    Vec::new()
}

/// Copy consecutive redirection wordcodes into a new Eprog.
/// Direct port of zsh/Src/parse.c:3003 `eccopyredirs`.
/// zshrs no-op.
pub fn eccopyredirs() -> Option<ZshProgram> {
    None
}

/// Initialize the dummy Eprog used as a placeholder. Direct port
/// of zsh/Src/parse.c:3069 `init_eprog`. zshrs no-op since
/// the AST has no equivalent dummy node — empty programs are
/// just `ZshProgram { lists: vec![] }`.
pub fn init_eprog() {
    // parse.c:3071-3074 — set up dummy_eprog_code = WCB_END().
    // zshrs no-op.
}

/// Parse the complete input. Direct port of `parse_event` /
/// `par_list` from `Src/parse.c:614-720`. On syntax error,
/// sets `errflag |= ERRFLAG_ERROR` (via `zerr`) and returns the
/// partial program — callers check `errflag` to detect failure,
/// matching C's `Eprog parse_event(...)` + `if (errflag) {...}`.
pub fn parse() -> ZshProgram {
    crate::ported::lex::zshlex();

    let mut program = parse_program_until(None);

    // Surface lexer-level errors (unmatched quote/heredoc/etc.)
    // that the parser silently rolls past. zsh aborts with a
    // diagnostic via `zerr` which sets `errflag |= ERRFLAG_ERROR`.
    if let Some(msg) = crate::ported::lex::error() {
        crate::ported::utils::zerr(&msg);
    }

    // Post-pass: wire heredoc bodies (collected by lexer.process_heredocs)
    // back into ZshRedir.heredoc fields via heredoc_idx.
    let bodies: Vec<HereDocInfo> = crate::ported::lex::heredocs_clone()
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

/// P9c: wordcode-emission parser entry. Direct port of zsh's
/// `parse_event(int endtok)` from `Src/parse.c:683-720`. Emits a
/// minimal wordcode stream for the parsed program into the live
/// `ECBUF` thread_local via P9b's `ecadd` / `ecstrcode` API and
/// returns the start index of the emitted Eprog (matching C's
/// `Eprog parse_event(...)` return).
///
/// Minimal implementation: emits `WCB_END()` only for now (P9c
/// stub). The full par_event/par_list/par_sublist/par_pipe/par_cmd
/// recursion that walks the token stream and emits the right
/// wordcode for each production is the multi-week rewrite called
/// out in PORT_PLAN.md. This stub establishes the entry point and
/// drives the live ECBUF emission so downstream consumers (P9d
/// exec_wordcode) have a real wordcode buffer to walk.
pub fn par_event_wordcode() -> usize {
    let start = ECUSED.get() as usize;
    // parse.c:691-710 — par_list loop. Each iteration emits one WC_LIST
    // entry plus its sublist payload; terminator handling between
    // lists matches the SEMI/NEWLIN/AMPER/SEPER switch in the C source.
    while crate::ported::lex::tok() != ENDINPUT
        && crate::ported::lex::tok() != LEXERR
    {
        par_list_wordcode();
        match crate::ported::lex::tok() {
            SEMI | NEWLIN | AMPER | AMPERBANG | SEPER => {
                crate::ported::lex::zshlex();
            }
            _ => break,
        }
    }
    // parse.c:712 — `ecadd(WCB_END());`
    ecadd(crate::ported::zsh_h::WCB_END());
    start
}

/// P9c stub: direct port of `par_list(int *complex)` from
/// `Src/parse.c:799-878`. Emits WC_LIST + the sublist body. Real
/// implementation tracks Z_SYNC/Z_ASYNC/Z_END flags and emits per-list
/// `WCB_LIST(flags, skip)` headers; this stub emits Z_SYNC + skip
/// patched after the sublist payload size is known.
pub fn par_list_wordcode() {
    let p = ecadd(0); // reserve WCB_LIST patch slot
    par_sublist_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            let data = (crate::ported::zsh_h::Z_SYNC as u32)
                | ((skip as u32) << crate::ported::zsh_h::WC_LIST_FREE);
            b[p] = crate::ported::zsh_h::WC_LIST
                | (data << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_sublist(int *complex)` from
/// `Src/parse.c:880-980`. Emits WC_SUBLIST + pipeline. Real
/// implementation handles `!`/`&&`/`||`/coproc; this stub emits a
/// single sublist + pipeline.
pub fn par_sublist_wordcode() {
    let p = ecadd(0);
    par_pipe_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            let data = (skip as u32) << 7;
            b[p] = crate::ported::zsh_h::WC_SUBLIST
                | (data << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_pline(int *complex)` from
/// `Src/parse.c:982-1020`. Emits WC_PIPE + cmd. Real implementation
/// handles `|`/`|&` pipeline chains.
pub fn par_pipe_wordcode() {
    let p = ecadd(0);
    par_cmd_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            let data = (crate::ported::zsh_h::WC_PIPE_END as u32)
                | ((skip as u32) << 1);
            b[p] = crate::ported::zsh_h::WC_PIPE
                | (data << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c: direct port of `par_cmd(int *complex, int zsh_construct)` from
/// `Src/parse.c:1022-1320`. Dispatches on the current token to the
/// appropriate par_* subroutine. Each form emits its WC_* opcode +
/// payload; default falls through to par_simple_wordcode.
pub fn par_cmd_wordcode() {
    match crate::ported::lex::tok() {
        FOR | FOREACH => par_for_wordcode(),
        SELECT => par_select_wordcode(),
        CASE => par_case_wordcode(),
        IF => par_if_wordcode(),
        WHILE => par_while_wordcode(),
        UNTIL => par_until_wordcode(),
        REPEAT => par_repeat_wordcode(),
        FUNC => par_funcdef_wordcode(),
        INPAR_TOK => par_subsh_wordcode(),
        INBRACE_TOK => par_cursh_wordcode(),
        TIME => par_time_wordcode(),
        DINBRACK => par_cond_wordcode(),
        DINPAR => par_arith_wordcode(),
        _ => par_simple_wordcode(),
    }
}

/// P9c stub: direct port of `par_for(int *complex)` from
/// `Src/parse.c:1641-1820`. Emits WC_FOR header + var-name strcode +
/// list-words + body wordcode. Stub form: emit WC_FOR header with
/// zero skip + zero param-name slot; real implementation parses
/// `for VAR in WORDS; do BODY; done`.
pub fn par_for_wordcode() {
    crate::ported::lex::zshlex(); // consume FOR
    let p = ecadd(0);
    ecadd(0); // param-name strcode placeholder
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_FOR
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: `par_select`. Direct port shape of Src/parse.c:1822-1860.
pub fn par_select_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    ecadd(0);
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_SELECT
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_case` from
/// `Src/parse.c:1862-1990`. Emits WC_CASE + word + arms.
pub fn par_case_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_CASE
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_if` from
/// `Src/parse.c:1992-2090`. Emits WC_IF + cond + then + elif chain.
pub fn par_if_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_IF
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_while` from
/// `Src/parse.c:2092-2150`. Emits WC_WHILE + cond + body.
pub fn par_while_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_WHILE
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: WC_WHILE with the until flag set on the header.
pub fn par_until_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            // WC_WHILE with type bit set = UNTIL (parse.c:2105 sets
            // type = WC_WHILE_UNTIL).
            let data = 1u32 | ((skip as u32) << 1);
            b[p] = crate::ported::zsh_h::WC_WHILE
                | (data << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_repeat` from
/// `Src/parse.c:2152-2200`. Emits WC_REPEAT + count strcode + body.
pub fn par_repeat_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_REPEAT
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_funcdef` from
/// `Src/parse.c:2202-2310`. Emits WC_FUNCDEF + name-list strcodes +
/// body. Real implementation parses multi-name funcdefs; stub emits
/// a header for a single-name body.
pub fn par_funcdef_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_FUNCDEF
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_subsh` for `(...)` subshell.
pub fn par_subsh_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_SUBSH
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_cursh` for `{...}` brace group.
pub fn par_cursh_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_list_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_CURSH
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_time` for `time` reserved-word.
pub fn par_time_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_pipe_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_TIMED
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_cond` for `[[ ... ]]` cond expression.
pub fn par_cond_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    par_simple_wordcode();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            let skip = b.len() - p - 1;
            b[p] = crate::ported::zsh_h::WC_COND
                | ((skip as u32) << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_arith` for `(( ... ))` arith block.
pub fn par_arith_wordcode() {
    crate::ported::lex::zshlex();
    let p = ecadd(0);
    let expr = crate::ported::lex::tokstr().unwrap_or_default();
    let coded = ecstrcode(&expr);
    ecadd(coded);
    crate::ported::lex::zshlex();
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            b[p] = crate::ported::zsh_h::WC_ARITH
                | (1u32 << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
}

/// P9c stub: direct port of `par_simple(int *complex, int nr)` from
/// `Src/parse.c:1321-1640`. Emits WC_SIMPLE + word count + interned
/// string offsets. Real implementation walks assignments + redirections
/// inline; this stub emits a WC_SIMPLE header with `nwords` plus one
/// ecstrcode'd string per STRING_LEX token consumed.
pub fn par_simple_wordcode() {
    let p = ecadd(0);
    let mut nwords: u32 = 0;
    while crate::ported::lex::tok() == STRING_LEX {
        let s = crate::ported::lex::tokstr().unwrap_or_default();
        let coded = ecstrcode(&s);
        ecadd(coded);
        nwords += 1;
        crate::ported::lex::zshlex();
    }
    ECBUF.with_borrow_mut(|b| {
        if p < b.len() {
            b[p] = crate::ported::zsh_h::WC_SIMPLE
                | (nwords << crate::ported::zsh_h::WC_CODEBITS);
        }
    });
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
        if check_limit() {
            error("parser exceeded global iteration limit");
            break;
        }

        // Skip separators
        while crate::ported::lex::tok() == SEPER || crate::ported::lex::tok() == NEWLIN {
            if check_limit() {
                error("parser exceeded global iteration limit");
                return ZshProgram { lists };
            }
            crate::ported::lex::zshlex();
        }

        if crate::ported::lex::tok() == ENDINPUT || crate::ported::lex::tok() == LEXERR {
            break;
        }

        // Check for end tokens
        if let Some(end_toks) = end_tokens {
            if end_toks.contains(&crate::ported::lex::tok()) {
                break;
            }
        }

        // Also stop at these tokens when not explicitly looking for them
        // Note: Else/Elif/Then are NOT here - they're handled by par_if
        // to allow nested if statements inside case arms, loops, etc.
        match crate::ported::lex::tok() {
            OUTBRACE_TOK | DSEMI | SEMIAMP | SEMIBAR | DONE | FI | ESAC | ZEND => break,
            _ => {}
        }

        match par_list() {
            Some(list) => {
                let detected = simple_name_with_inoutpar(&list);
                lists.push(list);
                // Synthesize a FuncDef for the `name() { body }` shape
                // at parse time so body_source is captured while the
                // lexer still has the input. The lexer port emits
                // `name(` as a single Word ending in `<INPAR><OUTPAR>`,
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
                                    lineno: crate::ported::lex::lineno(),
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
                                    lineno: crate::ported::lex::lineno(),
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
                    while crate::ported::lex::tok() == SEPER || crate::ported::lex::tok() == NEWLIN
                    {
                        crate::ported::lex::zshlex();
                    }
                    if crate::ported::lex::tok() == INBRACE_TOK {
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
                        let body_start = crate::ported::lex::pos();
                        crate::ported::lex::zshlex();
                        let body = parse_program();
                        let body_end = if crate::ported::lex::tok() == OUTBRACE_TOK {
                            crate::ported::lex::pos().saturating_sub(1)
                        } else {
                            crate::ported::lex::pos()
                        };
                        let body_source = crate::ported::lex::input_slice(body_start, body_end)
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty());
                        if crate::ported::lex::tok() == OUTBRACE_TOK {
                            crate::ported::lex::zshlex();
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
                                    lineno: crate::ported::lex::lineno(),
                                    merge_stderr: false,
                                },
                                next: None,
                                flags: SublistFlags::default(),
                            },
                            flags: ListFlags::default(),
                        };
                        lists.push(synthetic);
                    } else if !matches!(
                        crate::ported::lex::tok(),
                        ENDINPUT | OUTBRACE_TOK | SEPER | NEWLIN
                    ) {
                        // No-brace one-line body: `foo() echo hello`.
                        // Parse a single command for the body.
                        let body_cmd = par_cmd();
                        if let Some(cmd) = body_cmd {
                            let body_list = ZshList {
                                sublist: ZshSublist {
                                    pipe: ZshPipe {
                                        cmd,
                                        next: None,
                                        lineno: crate::ported::lex::lineno(),
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
                                        lineno: crate::ported::lex::lineno(),
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
            }
            None => break,
        }
    }

    ZshProgram { lists }
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
fn par_list() -> Option<ZshList> {
    let sublist = par_sublist()?;

    let flags = match crate::ported::lex::tok() {
        AMPER => {
            crate::ported::lex::zshlex();
            ListFlags {
                async_: true,
                disown: false,
            }
        }
        AMPERBANG => {
            crate::ported::lex::zshlex();
            ListFlags {
                async_: true,
                disown: true,
            }
        }
        SEPER | SEMI | NEWLIN => {
            crate::ported::lex::zshlex();
            ListFlags::default()
        }
        _ => ListFlags::default(),
    };

    Some(ZshList { sublist, flags })
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
    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() + 1);
    if check_recursion() {
        error("par_sublist: max recursion depth exceeded");
        PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
        return None;
    }

    let mut flags = SublistFlags::default();

    // Handle coproc and !
    if crate::ported::lex::tok() == COPROC {
        flags.coproc = true;
        crate::ported::lex::zshlex();
    } else if crate::ported::lex::tok() == BANG_TOK {
        flags.not = true;
        crate::ported::lex::zshlex();
    }

    let pipe = match par_pline() {
        Some(p) => p,
        None => {
            PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
            return None;
        }
    };

    // Check for && or ||
    let next = match crate::ported::lex::tok() {
        DAMPER => {
            crate::ported::lex::zshlex();
            skip_separators();
            par_sublist().map(|s| (SublistOp::And, Box::new(s)))
        }
        DBAR => {
            crate::ported::lex::zshlex();
            skip_separators();
            par_sublist().map(|s| (SublistOp::Or, Box::new(s)))
        }
        _ => None,
    };

    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
    Some(ZshSublist { pipe, next, flags })
}

/// Parse a pipeline
/// Parse a pipeline (cmds joined by `|` / `|&`). Direct port of
/// zsh/Src/parse.c:894 `par_pline`. AST: ZshPipe { cmds: Vec<ZshCommand> }.
/// C emits WC_PIPE wordcodes per command; same flow.
fn par_pline() -> Option<ZshPipe> {
    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() + 1);
    if check_recursion() {
        error("par_pline: max recursion depth exceeded");
        PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
        return None;
    }

    let lineno = crate::ported::lex::toklineno();
    let cmd = match par_cmd() {
        Some(c) => c,
        None => {
            PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
            return None;
        }
    };

    // Check for | or |&
    let mut merge_stderr = false;
    let next = match crate::ported::lex::tok() {
        BAR_TOK | BARAMP => {
            merge_stderr = crate::ported::lex::tok() == BARAMP;
            crate::ported::lex::zshlex();
            skip_separators();
            par_pline().map(Box::new)
        }
        _ => None,
    };

    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
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
/// INPAR subshell / INBRACE current-shell / TIME / NOCORRECT,
/// else simple). Direct port of zsh/Src/parse.c:958 `par_cmd`.
fn par_cmd() -> Option<ZshCommand> {
    // Parse leading redirections
    let mut redirs = Vec::new();
    while IS_REDIROP(crate::ported::lex::tok()) {
        if let Some(redir) = par_redir() {
            redirs.push(redir);
        }
    }

    let cmd = match crate::ported::lex::tok() {
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
        while IS_REDIROP(crate::ported::lex::tok()) {
            if let Some(redir) = par_redir() {
                trailing.push(redir);
            }
        }
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

    None
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
    const MAX_ITERATIONS: usize = 10_000;
    let mut iterations = 0;

    // Parse leading assignments
    while crate::ported::lex::tok() == ENVSTRING || crate::ported::lex::tok() == ENVARRAY {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            error("par_simple: exceeded max iterations in assignments");
            return None;
        }
        if let Some(assign) = parse_assign() {
            assigns.push(assign);
        }
        crate::ported::lex::zshlex();
    }

    // Parse words and redirections
    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            error("par_simple: exceeded max iterations");
            return None;
        }
        match crate::ported::lex::tok() {
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
                            format!("{}=({})", assign.name, elems.join(" "))
                        }
                    };
                    words.push(synthetic);
                }
                crate::ported::lex::zshlex();
            }
            STRING_LEX | TYPESET => {
                let s = crate::ported::lex::tokstr();
                if let Some(s) = s {
                    words.push(s);
                }
                crate::ported::lex::zshlex();
                // Check for function definition foo() { ... }
                if words.len() == 1 && peek_inoutpar() {
                    return parse_inline_funcdef(words.pop().unwrap());
                }
                // `{name}>file` named-fd redirect: the lexer doesn't
                // recognize this shape, so the bare word `{name}`
                // arrives as a String. If it matches `{IDENT}` and
                // the NEXT token is a redirop, pop it off as the
                // varid for that redir.
                if !words.is_empty() && IS_REDIROP(crate::ported::lex::tok()) {
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
            _ if IS_REDIROP(crate::ported::lex::tok()) => {
                match par_redir() {
                    Some(redir) => redirs.push(redir),
                    None => break, // Error in redir parsing, stop
                }
            }
            INOUTPAR if !words.is_empty() => {
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

/// Parse an assignment
/// Parse an assignment word `NAME=value` or `NAME=(arr items)`.
/// Sub-routine of par_simple. The C source handles assignments
/// inline in par_simple via the ENVSTRING/ENVARRAY token paths
/// (parse.c:1842-2000ish); zshrs splits it out to a dedicated
/// helper for clarity.
fn parse_assign() -> Option<ZshAssign> {
    // Helper: locate the EQUALS-marker that delimits NAME from
    // VALUE in an assignment-shaped tokstr. The lexer META-encodes
    // EVERY `=` (including those inside `${var%%=foo}` strip
    // patterns or `[idx]=...` subscripts), so a naive
    // `tokstr.find(EQUALS)` would split at the first inner `=`
    // and break the whole assignment. Walk the string skipping
    // brace and bracket depth so the assignment's `=` (the one
    // after the last `]` of the LHS subscript / or after the
    // bare name) is the one we land on.
    fn find_assign_equals(s: &str) -> Option<usize> {
        let target = crate::ported::zsh_h::EQUALS;
        let mut brace = 0i32;
        let mut bracket = 0i32;
        let mut paren = 0i32;
        for (i, c) in s.char_indices() {
            match c {
                    '{' | '\u{8f}' /* INBRACE */ => brace += 1,
                    '}' | '\u{90}' /* OUTBRACE */ => {
                        if brace > 0 {
                            brace -= 1;
                        }
                    }
                    '[' | '\u{91}' /* INBRACK */ => bracket += 1,
                    ']' | '\u{92}' /* OUTBRACK */ => {
                        if bracket > 0 {
                            bracket -= 1;
                        }
                    }
                    '(' | '\u{88}' /* INPAR */ => paren += 1,
                    ')' | '\u{8a}' /* OUTPAR */ => {
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

    let _ts_tokstr = crate::ported::lex::tokstr()?;
    let tokstr = _ts_tokstr.as_str();

    // Parse name=value or name+=value.
    let (name, value_str, append) = if crate::ported::lex::tok() == ENVARRAY {
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
            tokstr[pos + EQUALS.len_utf8()..].to_string(),
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

    let value = if crate::ported::lex::tok() == ENVARRAY {
        // Array assignment: name=(...)
        let mut elements = Vec::new();
        crate::ported::lex::zshlex(); // skip past token

        let mut arr_iters = 0;
        const MAX_ARRAY_ELEMENTS: usize = 10_000;
        while matches!(crate::ported::lex::tok(), STRING_LEX | SEPER | NEWLIN) {
            arr_iters += 1;
            if arr_iters > MAX_ARRAY_ELEMENTS {
                error("array assignment exceeded maximum elements");
                break;
            }
            if crate::ported::lex::tok() == STRING_LEX {
                let _ts_s = crate::ported::lex::tokstr();
                if let Some(s) = _ts_s.as_deref() {
                    elements.push(s.to_string());
                }
            }
            crate::ported::lex::zshlex();
        }

        // The closing OUTPAR is consumed here. The outer par_simple
        // loop will then `zshlex()` past whatever follows (typically
        // a separator or the next word) — calling zshlex twice in
        // tandem (here AND in par_simple) over-advances and merges
        // a following `name() { … }` funcdef into the same Simple.
        // We only consume Outpar; let the caller handle the rest.
        // Without this guard `g=(o1); f() { :; }` parsed as one
        // Simple with assigns=[g] and words=["f()"] (one token).
        if crate::ported::lex::tok() == OUTPAR_TOK {
            // Note: do NOT zshlex() here. par_simple's `lexer
            // .zshlex()` after `parse_assign` returns advances past
            // the Outpar onto the next significant token.
            //
            // Force `incmdpos=true` so the next zshlex() recognizes
            // a follow-up `b=(...)` / `b=val` as Envarray/Envstring.
            // The lexer flips incmdpos to false on bare Outpar (which
            // is correct for subshell-close context), but for an
            // array-assignment close more assigns/words may follow.
            crate::ported::lex::set_incmdpos(true);
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

/// Parse a redirection
/// Parse a redirection (>file, <file, >>file, <<HEREDOC, etc.).
/// Direct port of zsh/Src/parse.c:2229 `par_redir`. Returns
/// a ZshRedir node carrying the operator type, fd, target word
/// (or here-doc body / pipe-redir command), and any `{var}` style
/// fd-binding parameter.
fn par_redir() -> Option<ZshRedir> {
    let rtype = match crate::ported::lex::tok() {
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

    let fd = if crate::ported::lex::tokfd() >= 0 {
        crate::ported::lex::tokfd()
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

    crate::ported::lex::zshlex();

    let name = match crate::ported::lex::tok() {
        STRING_LEX | ENVSTRING => {
            let n = crate::ported::lex::tokstr().unwrap_or_default();
            crate::ported::lex::zshlex();
            n
        }
        _ => {
            error("expected word after redirection");
            return None;
        }
    };

    // Heredoc body capture: when reading the terminator above, the
    // lexer pushed a HereDoc to heredocs[]. Record the
    // index so fill_heredoc_bodies() can wire content back after
    // process_heredocs() has run.
    let heredoc_idx = if matches!(rtype, REDIR_HEREDOC | REDIR_HEREDOCDASH) {
        if !crate::ported::lex::heredocs_is_empty() {
            Some(crate::ported::lex::heredocs_len() - 1)
        } else {
            None
        }
    } else {
        None
    };

    Some(ZshRedir {
        rtype,
        fd,
        name,
        heredoc: None,
        varid: None,
        heredoc_idx,
    })
}

/// Parse for/foreach loop
/// Parse `for NAME in WORDS; do BODY; done` (foreach style) AND
/// `for ((init; cond; incr)) do BODY done` (c-style). Direct port
/// of zsh/Src/parse.c:1087 `par_for`. parse_for_cstyle is the
/// inner branch for the `((...))` arithmetic-header variant
/// (parse.c:1100-1140 inside par_for).
fn par_for() -> Option<ZshCommand> {
    let is_foreach = crate::ported::lex::tok() == FOREACH;
    crate::ported::lex::zshlex();

    // Check for C-style: for (( init; cond; step ))
    if crate::ported::lex::tok() == DINPAR {
        return parse_for_cstyle();
    }

    // Get variable name(s). zsh parse.c par_for accepts multiple
    // identifier tokens before `in`/`(`/newline — `for k v in ...`
    // assigns each iteration's pair of values to k and v in turn.
    // We store the names space-joined since variable identifiers
    // can't contain whitespace.
    let mut names: Vec<String> = Vec::new();
    while crate::ported::lex::tok() == STRING_LEX {
        let v = crate::ported::lex::tokstr().unwrap_or_default();
        if v == "in" {
            break;
        }
        names.push(v);
        crate::ported::lex::zshlex();
    }
    if names.is_empty() {
        error("expected variable name in for");
        return None;
    }
    let var = names.join(" ");

    // Skip newlines
    skip_separators();

    // Get list. The lexer-port quirk: `for x (a b c)` arrives as a
    // single String token with the parens lexed-as-content
    // (`<INPAR>a b c<OUTPAR>`) instead of as separate Inpar/String/
    // Outpar tokens. Detect that shape and split it manually.
    let list = if crate::ported::lex::tok() == STRING_LEX
        && crate::ported::lex::tokstr()
            .map(|s| s.starts_with('\u{88}') && s.ends_with('\u{8a}'))
            .unwrap_or(false)
    {
        let raw = crate::ported::lex::tokstr().unwrap_or_default();
        // Strip leading INPAR + trailing OUTPAR, then untokenize the
        // inner content and split on whitespace for the word list.
        let inner = &raw[raw.char_indices().nth(1).map(|(i, _)| i).unwrap_or(0)
            ..raw
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(raw.len())];
        let cleaned = super::lex::untokenize(inner);
        let words: Vec<String> = cleaned.split_whitespace().map(|s| s.to_string()).collect();
        crate::ported::lex::zshlex();
        ForList::Words(words)
    } else if crate::ported::lex::tok() == STRING_LEX {
        let s = crate::ported::lex::tokstr();
        if s.map(|s| s == "in").unwrap_or(false) {
            crate::ported::lex::zshlex();
            let mut words = Vec::new();
            let mut word_count = 0;
            while crate::ported::lex::tok() == STRING_LEX {
                word_count += 1;
                if word_count > 500 || check_limit() {
                    error("for: too many words");
                    return None;
                }
                let _ts_s = crate::ported::lex::tokstr();
                if let Some(s) = _ts_s.as_deref() {
                    words.push(s.to_string());
                }
                crate::ported::lex::zshlex();
            }
            ForList::Words(words)
        } else {
            ForList::Positional
        }
    } else if crate::ported::lex::tok() == INPAR_TOK {
        // for var (...)
        crate::ported::lex::zshlex();
        let mut words = Vec::new();
        let mut word_count = 0;
        while crate::ported::lex::tok() == STRING_LEX || crate::ported::lex::tok() == SEPER {
            word_count += 1;
            if word_count > 500 || check_limit() {
                error("for: too many words in parens");
                return None;
            }
            if crate::ported::lex::tok() == STRING_LEX {
                let _ts_s = crate::ported::lex::tokstr();
                if let Some(s) = _ts_s.as_deref() {
                    words.push(s.to_string());
                }
            }
            crate::ported::lex::zshlex();
        }
        if crate::ported::lex::tok() == OUTPAR_TOK {
            // After the `)` of a for-list, the next token is the
            // body opener — `do`/`{`. zsh's lexer needs incmdpos
            // set so `{` lexes as Inbrace (not as a literal). C
            // analogue: parse.c::par_for sets `incmdpos = 1`
            // after consuming the OUTPAR before the body parse.
            crate::ported::lex::set_incmdpos(true);
            crate::ported::lex::zshlex();
        }
        ForList::Words(words)
    } else {
        ForList::Positional
    };

    // Skip to body
    skip_separators();

    // Parse body
    let body = parse_loop_body(is_foreach)?;

    Some(ZshCommand::For(ZshFor {
        var,
        list,
        body: Box::new(body),
        is_select: false,
    }))
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

    crate::ported::lex::zshlex(); // Get init: Dinpar "i=0"

    if crate::ported::lex::tok() != DINPAR {
        error("expected init expression in for ((");
        return None;
    }
    let init = crate::ported::lex::tokstr().unwrap_or_default();

    crate::ported::lex::zshlex(); // Get cond: Dinpar "i<10"

    if crate::ported::lex::tok() != DINPAR {
        error("expected condition in for ((");
        return None;
    }
    let cond = crate::ported::lex::tokstr().unwrap_or_default();

    crate::ported::lex::zshlex(); // Get step: Doutpar "i++"

    if crate::ported::lex::tok() != DOUTPAR {
        error("expected )) in for");
        return None;
    }
    let step = crate::ported::lex::tokstr().unwrap_or_default();

    crate::ported::lex::zshlex(); // Move past ))

    skip_separators();
    let body = parse_loop_body(false)?;

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

/// Parse case statement
/// Parse `case WORD in PATTERN) BODY ;; ... esac`. Direct port
/// of zsh/Src/parse.c:1209 `par_case`. Each case arm is a
/// (pattern_list, body, terminator) tuple where terminator is
/// `;;` (default), `;&` (fallthrough), or `;|` (continue testing).
fn par_case() -> Option<ZshCommand> {
    crate::ported::lex::zshlex(); // skip 'case'

    let word = match crate::ported::lex::tok() {
        STRING_LEX => {
            let w = crate::ported::lex::tokstr().unwrap_or_default();
            crate::ported::lex::zshlex();
            w
        }
        _ => {
            error("expected word after case");
            return None;
        }
    };

    skip_separators();

    // Expect 'in' or {
    let use_brace = crate::ported::lex::tok() == INBRACE_TOK;
    if crate::ported::lex::tok() == STRING_LEX {
        let s = crate::ported::lex::tokstr();
        if s.map(|s| s != "in").unwrap_or(true) {
            error("expected 'in' in case");
            return None;
        }
    } else if !use_brace {
        error("expected 'in' or '{' in case");
        return None;
    }
    // Set incasepat=1 BEFORE consuming "in" so the next token (which
    // could be a leading `(` of a paren-prefixed pattern like
    // `case foo in (a|b) …`) is lexed as Inpar, not as a glob-token.
    // Without this the `(` got swallowed into a gettokstr('(', false)
    // call and produced a String like "(foo)" — the parser then saw
    // the `)` inside a string instead of as a separate Outpar.
    crate::ported::lex::set_incasepat(1);
    crate::ported::lex::zshlex();

    let mut arms = Vec::new();
    const MAX_ARMS: usize = 10_000;

    loop {
        if arms.len() > MAX_ARMS {
            error("par_case: too many arms");
            break;
        }

        // Set incasepat BEFORE skipping separators so lexer knows we're in case pattern context
        // This affects how [ and | are lexed
        crate::ported::lex::set_incasepat(1);

        skip_separators();

        // Check for end
        // Note: 'esac' might be String "esac" if incasepat > 0 prevents reserved word recognition
        let is_esac = crate::ported::lex::tok() == ESAC
            || (crate::ported::lex::tok() == STRING_LEX
                && crate::ported::lex::tokstr()
                    .map(|s| s == "esac")
                    .unwrap_or(false));
        if (use_brace && crate::ported::lex::tok() == OUTBRACE_TOK) || (!use_brace && is_esac) {
            crate::ported::lex::set_incasepat(0);
            crate::ported::lex::zshlex();
            break;
        }

        // Also break on EOF
        if crate::ported::lex::tok() == ENDINPUT || crate::ported::lex::tok() == LEXERR {
            crate::ported::lex::set_incasepat(0);
            break;
        }

        // Skip optional `(`. zsh's case grammar: `case W in (P)…)`.
        // The leading `(` is paired with a matching `)` that closes
        // the pattern itself; the arm-close `)` follows separately.
        // Track whether we consumed it so we can skip the matching
        // `)` after pattern parsing — otherwise the arm-close would
        // be interpreted as the pattern-close and the actual body
        // would get the leftover `)`.
        let had_leading_paren = crate::ported::lex::tok() == INPAR_TOK;
        if had_leading_paren {
            crate::ported::lex::zshlex();
        }

        // incasepat is already set above
        let mut patterns = Vec::new();
        let mut pattern_iterations = 0;
        loop {
            pattern_iterations += 1;
            if pattern_iterations > 1000 {
                error("par_case: too many pattern iterations");
                crate::ported::lex::set_incasepat(0);
                return None;
            }

            if crate::ported::lex::tok() == STRING_LEX {
                let s = crate::ported::lex::tokstr();
                if s.map(|s| s == "esac").unwrap_or(false) {
                    break;
                }
                patterns.push(crate::ported::lex::tokstr().unwrap_or_default());
                // After first pattern token, set incasepat=2 so ( is treated as part of pattern
                crate::ported::lex::set_incasepat(2);
                crate::ported::lex::zshlex();
            } else if crate::ported::lex::tok() != BAR_TOK {
                break;
            }

            if crate::ported::lex::tok() == BAR_TOK {
                // Reset to 1 (start of next alternative pattern)
                crate::ported::lex::set_incasepat(1);
                crate::ported::lex::zshlex();
            } else {
                break;
            }
        }
        crate::ported::lex::set_incasepat(0);

        // zsh's `(P)` form (parse.c:1320-1360 hack) treats the entire
        // parenthesized contents as ONE zsh pattern with internal `|`
        // as the literal alternation operator — NOT as multiple
        // case-arm alternatives. Without a leading `(`, the bare
        // `P1|P2)` form splits into multiple alts. Mirror that here:
        // when a leading `(` was consumed, fold the |-separated
        // pieces back into a single pattern string.
        if had_leading_paren && patterns.len() > 1 {
            let joined = patterns.join("|");
            patterns = vec![joined];
        }

        // Expect ).  Also handle the `(P))` wrapped-pattern form:
        // when a leading `(` was consumed, accept an extra `)` —
        // the inner `)` closes the optional-paren wrapper, the
        // outer `)` is the arm-close. zsh accepts BOTH `(P) BODY`
        // (bare pattern, leading-paren is just the opt-marker, the
        // close is arm-close) and `(P)) BODY` (paren-wrapped
        // pattern, then arm-close). The first form is unambiguous
        // when the bare pattern was simple; the second is needed
        // when the body starts with `(`.
        if crate::ported::lex::tok() != OUTPAR_TOK {
            error("expected ')' in case pattern");
            return None;
        }
        // Port of Src/parse.c:1310-1313 — when the case pattern
        // closes with `)`, set `incmdpos = 1` BEFORE consuming
        // the token so the first word of the arm body is lexed
        // in command position. Without this, `case X in X) c1=v ;;`
        // lexes `c1=v` as a plain STRING rather than an assignment
        // word, and exec treats it as a command name (yielding
        // "command not found: c1=v"). Subsequent statements after
        // `;` parse correctly because the `;` separator restores
        // command position; only the FIRST body word was broken.
        crate::ported::lex::set_incmdpos(true);
        crate::ported::lex::zshlex();
        if had_leading_paren && crate::ported::lex::tok() == OUTPAR_TOK {
            crate::ported::lex::set_incmdpos(true);
            crate::ported::lex::zshlex();
        }

        // Parse body
        let body = parse_program();

        // Get terminator. Set incasepat=1 BEFORE the zshlex
        // advance so the next token (the next arm's pattern, like
        // `[a-z]`) gets tokenized in pattern context. Without
        // this, a `[`-prefixed pattern after the FIRST arm became
        // Inbrack instead of String and the pattern-loop bailed
        // out with "expected ')' in case pattern".
        let terminator = match crate::ported::lex::tok() {
            DSEMI => {
                crate::ported::lex::set_incasepat(1);
                crate::ported::lex::zshlex();
                CaseTerm::Break
            }
            SEMIAMP => {
                crate::ported::lex::set_incasepat(1);
                crate::ported::lex::zshlex();
                CaseTerm::Continue
            }
            SEMIBAR => {
                crate::ported::lex::set_incasepat(1);
                crate::ported::lex::zshlex();
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
    crate::ported::lex::zshlex(); // skip 'if'

    // Parse condition - stops at 'then' or '{' (zsh allows { instead of then)
    let cond = Box::new(parse_program_until(Some(&[THEN, INBRACE_TOK])));

    skip_separators();

    // Expect 'then' or {
    let use_brace = crate::ported::lex::tok() == INBRACE_TOK;
    if crate::ported::lex::tok() != THEN && !use_brace {
        error("expected 'then' or '{' after if condition");
        return None;
    }
    crate::ported::lex::zshlex();

    // Parse then-body - stops at else/elif/fi, or } if using brace syntax
    let then = if use_brace {
        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
        if crate::ported::lex::tok() == OUTBRACE_TOK {
            crate::ported::lex::zshlex();
        }
        Box::new(body)
    } else {
        Box::new(parse_program_until(Some(&[ELSE, ELIF, FI])))
    };

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

    {
        loop {
            skip_separators();

            match crate::ported::lex::tok() {
                ELIF => {
                    crate::ported::lex::zshlex();
                    // elif condition stops at 'then' or '{'
                    let econd = parse_program_until(Some(&[THEN, INBRACE_TOK]));
                    skip_separators();

                    let elif_use_brace = crate::ported::lex::tok() == INBRACE_TOK;
                    if crate::ported::lex::tok() != THEN && !elif_use_brace {
                        error("expected 'then' after elif");
                        return None;
                    }
                    crate::ported::lex::zshlex();

                    // elif body stops at else/elif/fi or } if using braces
                    let ebody = if elif_use_brace {
                        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
                        if crate::ported::lex::tok() == OUTBRACE_TOK {
                            crate::ported::lex::zshlex();
                        }
                        body
                    } else {
                        parse_program_until(Some(&[ELSE, ELIF, FI]))
                    };

                    elif.push((econd, ebody));
                }
                ELSE => {
                    crate::ported::lex::zshlex();
                    skip_separators();

                    let else_use_brace = crate::ported::lex::tok() == INBRACE_TOK;
                    if else_use_brace {
                        crate::ported::lex::zshlex();
                    }

                    // else body stops at 'fi' or '}'
                    else_ = Some(Box::new(if else_use_brace {
                        let body = parse_program_until(Some(&[OUTBRACE_TOK]));
                        if crate::ported::lex::tok() == OUTBRACE_TOK {
                            crate::ported::lex::zshlex();
                        }
                        body
                    } else {
                        parse_program_until(Some(&[FI]))
                    }));

                    // Consume the 'fi' if present (not for brace syntax)
                    if !else_use_brace && crate::ported::lex::tok() == FI {
                        crate::ported::lex::zshlex();
                    }
                    break;
                }
                FI => {
                    crate::ported::lex::zshlex();
                    break;
                }
                _ => break,
            }
        }
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
    crate::ported::lex::zshlex(); // skip while/until

    let cond = Box::new(parse_program());

    skip_separators();
    let body = parse_loop_body(false)?;

    Some(ZshCommand::While(ZshWhile {
        cond,
        body: Box::new(body),
        until,
    }))
}

/// Parse repeat loop
/// Parse `repeat N; do BODY; done`. Direct port of
/// zsh/Src/parse.c:1565 `par_repeat`. The C source supports
/// the SHORTLOOPS short-form `repeat N CMD` (no do/done) — zshrs's
/// parser doesn't yet special-case that variant.
fn par_repeat() -> Option<ZshCommand> {
    crate::ported::lex::zshlex(); // skip 'repeat'

    let count = match crate::ported::lex::tok() {
        STRING_LEX => {
            let c = crate::ported::lex::tokstr().unwrap_or_default();
            crate::ported::lex::zshlex();
            c
        }
        _ => {
            error("expected count after repeat");
            return None;
        }
    };

    skip_separators();
    let body = parse_loop_body(false)?;

    Some(ZshCommand::Repeat(ZshRepeat {
        count,
        body: Box::new(body),
    }))
}

/// Parse loop body (do...done, {...}, or shortloop)
/// Parse the `do BODY done` body of a for/while/until/select/
/// repeat loop. Direct equivalent of zsh's parse.c handling
/// inside the loop builders — they all consume DOLOOP, parse a
/// list until DONE, and return the list. The `foreach_style`
/// flag signals foreach (where short-form `for NAME in WORDS;
/// CMD` may skip do/done) vs c-style (which always requires
/// do/done).
fn parse_loop_body(foreach_style: bool) -> Option<ZshProgram> {
    if crate::ported::lex::tok() == DOLOOP {
        crate::ported::lex::zshlex();
        let body = parse_program();
        if crate::ported::lex::tok() == DONE {
            crate::ported::lex::zshlex();
        }
        Some(body)
    } else if crate::ported::lex::tok() == INBRACE_TOK {
        crate::ported::lex::zshlex();
        let body = parse_program();
        if crate::ported::lex::tok() == OUTBRACE_TOK {
            crate::ported::lex::zshlex();
        }
        Some(body)
    } else if foreach_style {
        // foreach allows 'end' terminator
        let body = parse_program();
        if crate::ported::lex::tok() == ZEND {
            crate::ported::lex::zshlex();
        }
        Some(body)
    } else {
        // Short loop - single command
        par_list().map(|list| ZshProgram { lists: vec![list] })
    }
}

/// Parse (...) subshell
/// Parse a subshell `( ... )`. Direct port of zsh/Src/parse.c:1619
/// `par_subsh`. Body parses as a normal list; the subshell wrapper
/// fork-isolates execution in the executor.
fn par_subsh() -> Option<ZshCommand> {
    crate::ported::lex::zshlex(); // skip (
    let prog = parse_program();
    if crate::ported::lex::tok() == OUTPAR_TOK {
        crate::ported::lex::zshlex();
    }
    Some(ZshCommand::Subsh(Box::new(prog)))
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
    crate::ported::lex::zshlex(); // skip ()
    skip_separators();
    // No `{` after `()` → bare empty subshell shape `()`. Fall back
    // to a Subsh with an empty program so the status is 0 (matches
    // zsh's `()` no-op behavior).
    if crate::ported::lex::tok() != INBRACE_TOK {
        return Some(ZshCommand::Subsh(Box::new(ZshProgram {
            lists: Vec::new(),
        })));
    }
    crate::ported::lex::zshlex(); // skip {
    let body = parse_program();
    if crate::ported::lex::tok() == OUTBRACE_TOK {
        crate::ported::lex::zshlex();
    }
    // Collect any trailing args until a separator. zsh's anon-fn form
    // `() { body } a b c` runs body with $1=a, $2=b, $3=c.
    let mut args = Vec::new();
    while crate::ported::lex::tok() == STRING_LEX {
        if let Some(s) = crate::ported::lex::tokstr() {
            args.push(s);
        }
        crate::ported::lex::zshlex();
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
/// par_cmd at parse.c:958-1085 handles INBRACE → emit WC_CURSH
/// and recurses into the list. zshrs's parse_cursh extracts that
/// arm into a dedicated method.
fn parse_cursh() -> Option<ZshCommand> {
    crate::ported::lex::zshlex(); // skip {
    let prog = parse_program();

    // Check for { ... } always { ... }. Direct port of zsh's
    // par_subsh at parse.c:1612-1660 — note the two `incmdpos = 1`
    // forces (parse.c:1632, 1637): after consuming the closing
    // OUTBRACE AND after matching the `always` keyword, the parser
    // explicitly resets command position so the next `{` lexes as
    // INBRACE. Without these resets the lexer's String-clears-cmdpos
    // rule (lex.rs:976-983) leaves the second `{` in word position,
    // turning `always { ... }` into a Simple `{` `echo` … and the
    // try/always pairing is silently lost.
    if crate::ported::lex::tok() == OUTBRACE_TOK {
        crate::ported::lex::set_incmdpos(true); // parse.c:1632 incmdpos = !zsh_construct
        crate::ported::lex::zshlex();

        // Check for 'always'
        if crate::ported::lex::tok() == STRING_LEX {
            let s = crate::ported::lex::tokstr();
            if s.map(|s| s == "always").unwrap_or(false) {
                crate::ported::lex::set_incmdpos(true); // parse.c:1637 incmdpos = 1
                crate::ported::lex::zshlex();
                skip_separators();

                if crate::ported::lex::tok() == INBRACE_TOK {
                    crate::ported::lex::zshlex();
                    let always = parse_program();
                    if crate::ported::lex::tok() == OUTBRACE_TOK {
                        crate::ported::lex::zshlex();
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

/// Parse function definition
/// Parse `function NAME { BODY }` or `NAME () { BODY }`. Direct
/// port of zsh/Src/parse.c:1672 `par_funcdef`. zsh handles
/// the multiple keyword shapes (function FOO, FOO (), function FOO ()),
/// the optional `[fname1 fname2 ...]` for multi-name function defs,
/// and the `function FOO () { ... }` traditional/POSIX hybrid form.
fn par_funcdef() -> Option<ZshCommand> {
    crate::ported::lex::zshlex(); // skip 'function'

    let mut names = Vec::new();
    let mut tracing = false;

    // Handle options like -T and function names. Two subtleties:
    //
    //   1. Flags: zsh's lexer encodes a leading `-` as
    //      `zsh_h::DASH` (`\u{9b}`, `Src/zsh.h:182`) inside the String tokstr.
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
        match crate::ported::lex::tok() {
            STRING_LEX => {
                let _ts_s = crate::ported::lex::tokstr()?;
                let s = _ts_s.as_str();
                if s == "{" {
                    // Funcdef body opener — break, body-parser branch handles it.
                    break;
                }
                let first = s.chars().next();
                if matches!(first, Some('-') | Some('+')) || matches!(first, Some(c) if c == DASH) {
                    if s.contains('T') {
                        tracing = true;
                    }
                    crate::ported::lex::zshlex();
                    continue;
                }
                names.push(s.to_string());
                crate::ported::lex::zshlex();
            }
            INBRACE_TOK | INOUTPAR | SEPER | NEWLIN => break,
            _ => break,
        }
    }

    // Optional ()
    let saw_paren = crate::ported::lex::tok() == INOUTPAR;
    if saw_paren {
        crate::ported::lex::zshlex();
    }

    skip_separators();

    // Body opener: real Inbrace OR a String("{") (the lexer emits
    // the latter after a String NAME — see comment above).
    let body_opener_is_string_brace =
        crate::ported::lex::tok() == STRING_LEX && crate::ported::lex::tokstr_eq("{");
    if crate::ported::lex::tok() == INBRACE_TOK || body_opener_is_string_brace {
        // Capture body_start BEFORE the lexer advances past the
        // first body token. After the previous zshlex consumed
        // `{`, lexer.pos points just past `{` (which is where the
        // body source starts). The next `zshlex()` would advance
        // past the first token (`echo`), making body_start land
        // mid-body and lose the first word — `typeset -f f` would
        // print `a; echo b` for `{ echo a; echo b }`.
        let body_start = crate::ported::lex::pos();
        crate::ported::lex::zshlex();
        let body = parse_program();
        let body_end = if crate::ported::lex::tok() == OUTBRACE_TOK {
            // Lexer has just consumed `}`; pos is past it. Body content
            // ends one byte before pos.
            crate::ported::lex::pos().saturating_sub(1)
        } else {
            crate::ported::lex::pos()
        };
        let body_source = crate::ported::lex::input_slice(body_start, body_end)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if crate::ported::lex::tok() == OUTBRACE_TOK {
            crate::ported::lex::zshlex();
        }

        // Anonymous form `function () { body } a b c` (with `()`) or
        // `function { body } a b c` (zsh-only shorthand, no `()`). No
        // name was collected. Mirror parse_anon_funcdef: synthesize
        // `_zshrs_anon_N`, collect trailing args, set auto_call_args
        // so compile_funcdef registers + immediately calls the
        // function with the args as positional params.
        if names.is_empty() {
            let mut args = Vec::new();
            while crate::ported::lex::tok() == STRING_LEX {
                if let Some(s) = crate::ported::lex::tokstr() {
                    args.push(s);
                }
                crate::ported::lex::zshlex();
            }
            use std::sync::atomic::{AtomicUsize, Ordering};
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
        par_list().map(|list| {
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

/// Parse inline function definition: name() { ... }
/// Parse the inline form `NAME () { BODY }` (POSIX-style funcdef
/// without the `function` keyword). The name has already been
/// consumed and pushed by par_simple before this method fires.
/// C source: handled inline in par_simple's INOUTPAR-after-name
/// arm (parse.c:1836-2228).
fn parse_inline_funcdef(name: String) -> Option<ZshCommand> {
    // Skip ()
    if crate::ported::lex::tok() == INOUTPAR {
        crate::ported::lex::zshlex();
    }

    skip_separators();

    // Parse body
    if crate::ported::lex::tok() == INBRACE_TOK {
        // Same body_start-before-zshlex fix as par_funcdef.
        let body_start = crate::ported::lex::pos();
        crate::ported::lex::zshlex();
        let body = parse_program();
        let body_end = if crate::ported::lex::tok() == OUTBRACE_TOK {
            crate::ported::lex::pos().saturating_sub(1)
        } else {
            crate::ported::lex::pos()
        };
        let body_source = crate::ported::lex::input_slice(body_start, body_end)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if crate::ported::lex::tok() == OUTBRACE_TOK {
            crate::ported::lex::zshlex();
        }
        Some(ZshCommand::FuncDef(ZshFuncDef {
            names: vec![name],
            body: Box::new(body),
            tracing: false,
            auto_call_args: None,
            body_source,
        }))
    } else {
        match par_cmd() {
            Some(cmd) => {
                let list = ZshList {
                    sublist: ZshSublist {
                        pipe: ZshPipe {
                            cmd,
                            next: None,
                            lineno: crate::ported::lex::lineno(),
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

/// Parse [[ ... ]] conditional
/// Parse `[[ EXPR ]]` conditional expression. Direct port of
/// zsh/Src/parse.c:2409 `par_cond` (and helpers par_cond_1,
/// par_cond_2, par_cond_double, par_cond_triple, par_cond_multi
/// at parse.c:2434-2731). Expression operators: `||` `&&` `!`
/// + unary tests (-f, -d, -n, -z, etc.) + binary tests (=, !=,
///   <, >, ==, =~, -eq, -ne, -lt, -le, -gt, -ge, -nt, -ot, -ef).
fn par_cond() -> Option<ZshCommand> {
    crate::ported::lex::zshlex(); // skip [[
                                  // Empty cond `[[ ]]` is a parse error in zsh — emit the
                                  // diagnostic and return None so the caller produces a
                                  // non-zero exit. Without this, `[[ ]]` silently passed and
                                  // returned exit 0.
    if crate::ported::lex::tok() == DOUTBRACK {
        error("parse error near `]]'");
        crate::ported::lex::zshlex();
        return None;
    }
    let cond = parse_cond_expr();

    if crate::ported::lex::tok() == DOUTBRACK {
        crate::ported::lex::zshlex();
    }

    cond.map(ZshCommand::Cond)
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
    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() + 1);
    if check_recursion() {
        error("parse_cond_or: max recursion depth exceeded");
        PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
        return None;
    }

    let left = match parse_cond_and() {
        Some(l) => l,
        None => {
            PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
            return None;
        }
    };

    skip_cond_separators();

    let result = if crate::ported::lex::tok() == DBAR {
        crate::ported::lex::zshlex();
        skip_cond_separators();
        parse_cond_or().map(|right| ZshCond::Or(Box::new(left), Box::new(right)))
    } else {
        Some(left)
    };

    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
    result
}

/// Cond-expression `&&` level. C: par_cond_2 at parse.c:2476-2625.
fn parse_cond_and() -> Option<ZshCond> {
    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() + 1);
    if check_recursion() {
        error("parse_cond_and: max recursion depth exceeded");
        PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
        return None;
    }

    let left = match parse_cond_not() {
        Some(l) => l,
        None => {
            PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
            return None;
        }
    };

    skip_cond_separators();

    let result = if crate::ported::lex::tok() == DAMPER {
        crate::ported::lex::zshlex();
        skip_cond_separators();
        parse_cond_and().map(|right| ZshCond::And(Box::new(left), Box::new(right)))
    } else {
        Some(left)
    };

    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
    result
}

/// Cond-expression `!` negation level. C: handled inside
/// par_cond_2 at parse.c:2476-2625 via the BANG token check.
fn parse_cond_not() -> Option<ZshCond> {
    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() + 1);
    if check_recursion() {
        error("parse_cond_not: max recursion depth exceeded");
        PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
        return None;
    }

    skip_cond_separators();

    // ! can be either BANG_TOK or String "!"
    let is_not = crate::ported::lex::tok() == BANG_TOK
        || (crate::ported::lex::tok() == STRING_LEX
            && crate::ported::lex::tokstr()
                .map(|s| s == "!")
                .unwrap_or(false));
    if is_not {
        crate::ported::lex::zshlex();
        let inner = match parse_cond_not() {
            Some(i) => i,
            None => {
                PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
                return None;
            }
        };
        PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
        return Some(ZshCond::Not(Box::new(inner)));
    }

    if crate::ported::lex::tok() == INPAR_TOK {
        crate::ported::lex::zshlex();
        skip_cond_separators();
        let inner = match parse_cond_expr() {
            Some(i) => i,
            None => {
                PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
                return None;
            }
        };
        skip_cond_separators();
        if crate::ported::lex::tok() == OUTPAR_TOK {
            crate::ported::lex::zshlex();
        }
        PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
        return Some(inner);
    }

    let result = parse_cond_primary();
    PARSER_RECURSION_DEPTH.set(PARSER_RECURSION_DEPTH.get() - 1);
    result
}

/// Cond-expression primary: unary tests (-f, -d, ...), binary
/// tests (=, !=, <, >, ==, =~, -eq, -ne, ...), and parenthesized
/// sub-expressions. Direct port of par_cond_double / par_cond_triple
/// / par_cond_multi at parse.c:2626-2731 (chosen by arg count).
fn parse_cond_primary() -> Option<ZshCond> {
    let s1 = match crate::ported::lex::tok() {
        STRING_LEX => {
            let s = crate::ported::lex::tokstr().unwrap_or_default();
            crate::ported::lex::zshlex();
            s
        }
        _ => return None,
    };

    skip_cond_separators();

    // Check for unary operator. zsh's lexer tokenizes leading `-` as
    // `zsh_h::DASH` (`\u{9b}`, `Src/zsh.h:182`) inside gettokstr (lex.c:1390-1400
    // LX2_DASH — `-` always becomes Dash, untokenized later). Match
    // either form here, and use char-count not byte-count since DASH
    // is 2 UTF-8 bytes (`\xc2\x9b`).
    let s1_chars: Vec<char> = s1.chars().collect();
    if s1_chars.len() == 2 && IS_DASH(s1_chars[0]) {
        let s2 = match crate::ported::lex::tok() {
            STRING_LEX => {
                let s = crate::ported::lex::tokstr().unwrap_or_default();
                crate::ported::lex::zshlex();
                s
            }
            _ => return Some(ZshCond::Unary("-n".to_string(), s1)),
        };
        return Some(ZshCond::Unary(s1, s2));
    }

    // Check for binary operator. Direct port of zsh/Src/parse.c:2601-2603:
    //   incond++;  /* parentheses do globbing */
    //   do condlex(); while (COND_SEP());
    //   incond--;  /* parentheses do grouping */
    // The bump makes the lexer treat `(` as a literal character inside
    // the RHS word (e.g. `[[ x =~ (foo) ]]`) instead of returning INPAR
    // and splitting the regex into multiple tokens.
    let op = match crate::ported::lex::tok() {
        STRING_LEX => {
            let s = crate::ported::lex::tokstr().unwrap_or_default();
            crate::ported::lex::set_incond(crate::ported::lex::incond() + 1);
            crate::ported::lex::zshlex();
            crate::ported::lex::set_incond(crate::ported::lex::incond() - 1);
            s
        }
        INANG_TOK => {
            crate::ported::lex::set_incond(crate::ported::lex::incond() + 1);
            crate::ported::lex::zshlex();
            crate::ported::lex::set_incond(crate::ported::lex::incond() - 1);
            "<".to_string()
        }
        OUTANG_TOK => {
            crate::ported::lex::set_incond(crate::ported::lex::incond() + 1);
            crate::ported::lex::zshlex();
            crate::ported::lex::set_incond(crate::ported::lex::incond() - 1);
            ">".to_string()
        }
        _ => return Some(ZshCond::Unary("-n".to_string(), s1)),
    };

    skip_cond_separators();

    let s2 = match crate::ported::lex::tok() {
        STRING_LEX => {
            let s = crate::ported::lex::tokstr().unwrap_or_default();
            crate::ported::lex::zshlex();
            s
        }
        _ => return Some(ZshCond::Binary(s1, op, String::new())),
    };

    if op == "=~" {
        Some(ZshCond::Regex(s1, s2))
    } else {
        Some(ZshCond::Binary(s1, op, s2))
    }
}

fn skip_cond_separators() {
    while crate::ported::lex::tok() == SEPER && {
        let s = crate::ported::lex::tokstr();
        s.map(|s| !s.contains(';')).unwrap_or(true)
    } {
        crate::ported::lex::zshlex();
    }
}

/// Parse (( ... )) arithmetic command
/// Parse `(( EXPR ))` arithmetic command. C source: parse.c:1810-1834
/// `par_dinbrack` (despite the name; the function actually handles
/// DINPAR `(( ))` blocks too).
fn parse_arith() -> Option<ZshCommand> {
    let expr = crate::ported::lex::tokstr().unwrap_or_default();
    crate::ported::lex::zshlex();
    Some(ZshCommand::Arith(expr))
}

/// Parse time command
/// Parse `time CMD` (POSIX time keyword). Direct port of
/// zsh/Src/parse.c:1787 `par_time`. The `time` keyword
/// times the execution of the following pipeline / cmd.
fn par_time() -> Option<ZshCommand> {
    crate::ported::lex::zshlex(); // skip 'time'

    // Check if there's a pipeline to time
    if crate::ported::lex::tok() == SEPER
        || crate::ported::lex::tok() == NEWLIN
        || crate::ported::lex::tok() == ENDINPUT
    {
        Some(ZshCommand::Time(None))
    } else {
        let sublist = par_sublist();
        Some(ZshCommand::Time(sublist.map(Box::new)))
    }
}

/// Check if next token is ()
fn peek_inoutpar() -> bool {
    crate::ported::lex::tok() == INOUTPAR
}

/// Skip separator tokens
fn skip_separators() {
    let mut iterations = 0;
    while crate::ported::lex::tok() == SEPER || crate::ported::lex::tok() == NEWLIN {
        iterations += 1;
        if iterations > 100_000 {
            error("skip_separators: too many iterations");
            return;
        }
        crate::ported::lex::zshlex();
    }
}

/// Record a parse error. Direct port of zsh's `zerr` invocation
/// from `Src/parse.c:625-633 yyerror`. Sets `errflag |=
/// ERRFLAG_ERROR` (when `noerrs == 0`) and emits a diagnostic on
/// stderr via `zwarning`.
fn error(msg: &str) {
    crate::ported::utils::zerr(msg);
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

/// `#define FDHF_KSHLOAD 1` from `Src/parse.c:3149`. Function-header
/// flag word — `-k` ksh-style autoload marker.
pub const FDHF_KSHLOAD: u32 = 1;

/// `#define FDHF_ZSHLOAD 2` from `Src/parse.c:3150`. `-z` zsh-style
/// autoload marker.
pub const FDHF_ZSHLOAD: u32 = 2;

/// Port of `struct fdhead` from `Src/parse.c:3116`. One per function
/// inside a wordcode dump. All fields are `wordcode` (u32).
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
pub struct fdhead {
    /// Offset (in u32 words) to the start of this function's
    /// wordcode body inside the dump.
    pub start: u32,                                                          // c:3117
    /// Wordcode-byte length of the body (excludes pattern-prog slots).
    pub len: u32,                                                            // c:3118
    /// Number of compiled patterns the body references.
    pub npats: u32,                                                          // c:3119
    /// Offset of the string table inside `prog->prog`.
    pub strs: u32,                                                           // c:3120
    /// Header-record length in u32 words (record + name).
    pub hlen: u32,                                                           // c:3121
    /// Packed `{ kshload_bits(2) | name_tail_offset(30) }` field.
    pub flags: u32,                                                          // c:3122
}

/// Size of `struct fdhead` in `wordcode` (u32) units. Used by all
/// the header-walk macros below.
pub const FDHEAD_WORDS: usize = std::mem::size_of::<fdhead>() / 4;

/// Port of `struct wcfunc` from `Src/parse.c:3158`. Build-time
/// per-function aggregate before write_dump emits it. The Rust
/// port stores the source-text body inline since the C-side
/// `Eprog` ↔ `parse_string` chain isn't fully wired through this
/// layer yet (`build_dump` falls back to source-text caching).
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct wcfunc {
    pub name: String,                                                        // c:3159
    pub flags: u32,                                                          // c:3161
    /// Compiled body wordcode (one `u32` array per fn). Empty until
    /// the eprog emit-side lands; `write_dump` then walks each entry.
    pub body: Vec<u32>,
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
    let bytes: Vec<u8> = f[2..].iter().take(10)
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
    cur + (f[cur + 4] as usize)  // .hlen is field 4 of fdhead
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
    let bytes: Vec<u8> = buf[name_word_off..].iter()
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
        len:   buf[offset + 1],
        npats: buf[offset + 2],
        strs:  buf[offset + 3],
        hlen:  buf[offset + 4],
        flags: buf[offset + 5],
    })
}

/// Port of `fdswap(Wordcode p, int n)` from `Src/parse.c:3318`.
/// Byte-swap each u32 in `p[..n]` in place. Used when writing the
/// opposite-byte-order copy of a wordcode dump.
pub fn fdswap(p: &mut [u32]) {                                                // c:3318
    for w in p.iter_mut() {
        *w = w.swap_bytes();
    }
}

/// Port of `dump_find_func(Wordcode h, char *name)` from
/// `Src/parse.c:3167`. Walks the header table inside a loaded
/// dump for a function with the given basename; returns true on hit.
pub fn dump_find_func(h: &[u32], name: &str) -> bool {                        // c:3167
    let header_words = fdheaderlen(h) as usize;
    let end = header_words; // walking u32 offsets, end-exclusive
    let mut cur = firstfdhead_offset();
    while cur < end {
        if let Some(fh) = read_fdhead(h, cur) {
            let full = fdname(h, cur);
            let tail = fdhtail(&fh) as usize;
            let basename = if tail <= full.len() { &full[tail..] } else { "" };
            if basename == name {
                return true;
            }
            cur = nextfdhead_offset(h, cur);
        } else {
            break;
        }
    }
    false
}

/// Port of `load_dump_header(char *nam, char *name, int err)` from
/// `Src/parse.c:3258`. Opens the file, reads + validates the magic
/// and version, then slurps the full header table into memory.
/// Returns the header u32-array on success or None on any failure
/// (emitting C-shaped warnings when `err != 0`).
pub fn load_dump_header(nam: &str, name: &str, err: i32) -> Option<Vec<u32>> { // c:3258

    let mut f = match File::open(name) {                                      // c:3263
        Ok(h) => h,
        Err(_) => {
            if err != 0 {
                zwarnnam(nam, &format!("can't open zwc file: {}", name));     // c:3265
            }
            return None;
        }
    };

    // Read FD_PRELEN+1 u32 words = 52 bytes.
    let mut buf_bytes = vec![0u8; (FD_PRELEN + 1) * 4];
    if f.read_exact(&mut buf_bytes).is_err() {
        if err != 0 {
            zwarnnam(nam, &format!("invalid zwc file: {}", name));            // c:3277
        }
        return None;
    }
    let mut buf: Vec<u32> = buf_bytes.chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    // c:3270 — magic + version check. `ZSH_VERSION` (the C-side
    // global) — zshrs reports "5.9" in `--zsh` mode (Src/init.c parity).
    let magic_ok = fdmagic(&buf) == FD_MAGIC || fdmagic(&buf) == FD_OMAGIC;
    let v_ok = fdversion(&buf) == "5.9";
    if !magic_ok {
        if err != 0 {
            zwarnnam(nam, &format!("invalid zwc file: {}", name));            // c:3277
        }
        return None;
    }
    if !v_ok {
        if err != 0 {
            zwarnnam(nam, &format!("zwc file has wrong version (zsh-{}): {}", // c:3274
                                   fdversion(&buf), name));
        }
        return None;
    }

    // c:3285 — if magic matches host byte order, head len is `pre[FD_PRELEN]`.
    // Else seek to `fdother(buf)` and re-read.
    if fdmagic(&buf) != FD_MAGIC {
        let other = fdother(&buf) as u64;                                     // c:3290
        if f.seek(SeekFrom::Start(other)).is_err()
            || f.read_exact(&mut buf_bytes).is_err()
        {
            zwarnnam(nam, &format!("invalid zwc file: {}", name));            // c:3295
            return None;
        }
        buf = buf_bytes.chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
    }

    let total_words = fdheaderlen(&buf) as usize;                             // c:3286/3299
    if total_words < FD_PRELEN + 1 {
        zwarnnam(nam, &format!("invalid zwc file: {}", name));
        return None;
    }

    // Read the remaining header words.
    let mut head: Vec<u32> = Vec::with_capacity(total_words);
    head.extend_from_slice(&buf);
    let remaining_words = total_words - (FD_PRELEN + 1);
    if remaining_words > 0 {
        let mut rest_bytes = vec![0u8; remaining_words * 4];                  // c:3305
        if f.read_exact(&mut rest_bytes).is_err() {
            zwarnnam(nam, &format!("invalid zwc file: {}", name));            // c:3307
            return None;
        }
        for c in rest_bytes.chunks_exact(4) {
            head.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
    }
    Some(head)                                                                 // c:3311
}

/// Port of `build_dump(char *nam, char *dump, char **files, int ali, int map, int flags)`
/// from `Src/parse.c:3397`. Source-file → wordcode dump compiler.
///
/// Status: scaffolded but the wordcode-emit step depends on
/// `parse_string` returning a fully-wired `Eprog` with `prog/strs/
/// npats` fields populated. The current `parse_string`/`parse` shape
/// emits an AST (`ZshProgram`) but not yet the wordcode array C
/// expects in this dump format. Until that lands, this returns 1
/// with a clear "wordcode emit not yet ported" message so callers
/// (autoload from `.zwc`, `zcompile path/to/file`) fail loud.
pub fn build_dump(nam: &str,                                                  // c:3397
                  dump: &str,
                  _files: &[String],
                  _ali: i32,
                  _map: i32,
                  _flags: u32) -> i32 {
    crate::ported::utils::zwarnnam(nam, &format!("{}: wordcode dump emit not yet ported", dump));
    1
}

/// Port of `build_cur_dump(char *nam, char *dump, char **names, int match, int map, int what)`
/// from `Src/parse.c:3536`. Compiles currently-loaded functions
/// (`-c` for functions, `-a` for aliases) into a `.zwc` dump.
/// Same wordcode-emit dependency as `build_dump`.
pub fn build_cur_dump(nam: &str,                                              // c:3536
                      dump: &str,
                      _names: &[String],
                      _match_: i32,
                      _map: i32,
                      _what: i32) -> i32 {
    crate::ported::utils::zwarnnam(nam, &format!("{}: wordcode dump-current emit not yet ported", dump));
    1
}

/// Port of `zwcstat(char *filename, struct stat *buf)` from
/// `Src/parse.c:3656`. Stats a `.zwc` file, falling back to
/// `.zwc.old` if the primary doesn't exist (zsh uses the `.old`
/// suffix to keep a previous dump readable while a rewrite is in
/// progress).
pub fn zwcstat(filename: &str) -> Option<std::fs::Metadata> {                 // c:3656
    if let Ok(m) = std::fs::metadata(filename) {
        return Some(m);
    }
    let old = format!("{}.old", filename);
    std::fs::metadata(&old).ok()
}

/// Port of `load_dump_file(char *dump, struct stat *sbuf, int other, int len)`
/// from `Src/parse.c:3675`. Reads (or mmap()'s) a complete `.zwc`
/// file into memory. Returns the u32 buffer or None on I/O error.
pub fn load_dump_file(dump: &str,                                             // c:3675
                      _sbuf: &std::fs::Metadata,
                      other: i32,
                      _len: usize) -> Option<Vec<u32>> {
    let mut f = File::open(dump).ok()?;
    if other != 0 {
        f.seek(SeekFrom::Start(other as u64)).ok()?;
    }
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    Some(bytes.chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Port of `try_dump_file(char *path, char *name, char *file, int *ksh, int test_only)`
/// from `Src/parse.c:3746`. Tries to load a function from a `.zwc`
/// in the given fpath directory. Returns `(found, ksh_load)` —
/// stub: returns false until the dump-cache port (`FuncDump`) lands.
pub fn try_dump_file(_path: &str, _name: &str, _file: &str,                   // c:3746
                     _test_only: bool) -> Option<(bool, bool)> {
    None
}

/// Port of `try_source_file(char *file)` from `Src/parse.c:3795`.
/// Tries `source <file>` then falls back to `source <file>.zwc`.
/// Returns the resolved path on hit. Stub: returns None until the
/// dump-cache port lands.
pub fn try_source_file(_file: &str) -> Option<String> {                       // c:3795
    None
}

/// Port of `check_dump_file(char *file, struct stat *sbuf, char *name, int *ksh, int test_only)`
/// from `Src/parse.c:3833`. Opens + validates a `.zwc` file,
/// returning its loaded buffer or None.
pub fn check_dump_file(_file: &str,                                           // c:3833
                       _sbuf: &std::fs::Metadata,
                       _name: &str,
                       _test_only: bool) -> Option<(Vec<u32>, bool)> {
    None
}

/// Port of `incrdumpcount(FuncDump f)` from `Src/parse.c:3970/4021`.
/// Refcount-up a loaded `.zwc` dump. No-op stub.
pub fn incrdumpcount() {}                                                     // c:3970

/// Port of `decrdumpcount(FuncDump f)` from `Src/parse.c:3988/4026`.
/// Refcount-down + free if zero. No-op stub.
pub fn decrdumpcount() {}                                                     // c:3988

/// Port of `freedump(FuncDump f)` from `Src/parse.c:3976`. Releases
/// the per-dump memory + close fd. No-op stub.
pub fn freedump() {}                                                          // c:3976

/// Port of `closedumps(void)` from `Src/parse.c:4008/4033`. Walks
/// the `dumps` list and frees every entry. No-op stub.
pub fn closedumps() {}                                                        // c:4008

/// Port of `dump_autoload(char *nam, char *file, int on, Options ops, int func)`
/// from `Src/parse.c:4042`. Registers every function in a `.zwc`
/// for autoload via `shfunctab`. Stub: returns 1 (error) until the
/// dump-cache port lands.
pub fn dump_autoload(nam: &str, file: &str,                                   // c:4042
                     _on: i32,
                     _ops: &crate::ported::zsh_h::options,
                     _func: i32) -> i32 {
    crate::ported::utils::zwarnnam(nam, &format!("{}: zwc-based autoload not yet ported", file));
    1
}

/// Port of `bin_zcompile(char *nam, char **args, Options ops, UNUSED(int func))`
/// from `Src/parse.c:3180`. Validates the option set, then dispatches
/// to one of: `-t` (test/list), `-c`/`-a` (dump current functions),
/// or the default (compile source files to `.zwc`).
pub fn bin_zcompile(nam: &str,                                                // c:3180
                    args: &[String],
                    ops: &crate::ported::zsh_h::options,
                    _func: i32) -> i32 {

    // c:3185-3192 — illegal-combination guard.
    let bad_combo = (OPT_ISSET(ops, b'k') && OPT_ISSET(ops, b'z'))
        || (OPT_ISSET(ops, b'R') && OPT_ISSET(ops, b'M'))
        || (OPT_ISSET(ops, b'c')
            && (OPT_ISSET(ops, b'U') || OPT_ISSET(ops, b'k') || OPT_ISSET(ops, b'z')))
        || (!(OPT_ISSET(ops, b'c') || OPT_ISSET(ops, b'a'))
            && OPT_ISSET(ops, b'm'));
    if bad_combo {
        zwarnnam(nam, "illegal combination of options");                      // c:3192
        return 1;
    }

    // c:3194 — `-c`/`-a` + KSHAUTOLOAD warning.
    if (OPT_ISSET(ops, b'c') || OPT_ISSET(ops, b'a'))
        && crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHAUTOLOAD)
    {
        zwarnnam(nam, "functions will use zsh style autoloading");            // c:3195
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
    if OPT_ISSET(ops, b't') {                                                 // c:3199
        if args.is_empty() {
            zwarnnam(nam, "too few arguments");                               // c:3202
            return 1;
        }
        let dump_name = if args[0].ends_with(FD_EXT) {
            args[0].clone()
        } else {
            format!("{}{}", args[0], FD_EXT)
        };
        let f = match load_dump_header(nam, &dump_name, 1) {                  // c:3206
            Some(buf) => buf,
            None => return 1,
        };
        // c:3209 — per-function check.
        if args.len() > 1 {
            for name in &args[1..] {                                          // c:3210
                if !dump_find_func(&f, name) {                                // c:3212
                    return 1;
                }
            }
            return 0;
        }
        // c:3215-3221 — listing arm. Walk every fdhead, print
        // each function's full name. C uses `fdname(h)` which
        // includes the path prefix; matches our `fdname()` impl.
        let mapped = if (fdflags(&f) & FDF_MAP) != 0 { "mapped" } else { "read" };
        println!("zwc file ({}) for zsh-{}", mapped, fdversion(&f));
        let header_words = fdheaderlen(&f) as usize;
        let mut cur = firstfdhead_offset();
        while cur < header_words {
            if read_fdhead(&f, cur).is_none() { break; }
            println!("{}", fdname(&f, cur));
            cur = nextfdhead_offset(&f, cur);
        }
        return 0;
    }

    if args.is_empty() {
        zwarnnam(nam, "too few arguments");                                   // c:3226
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
        let what = (if OPT_ISSET(ops, b'c') { 1 } else { 0 })
            | (if OPT_ISSET(ops, b'a') { 2 } else { 0 });
        build_cur_dump(nam, &dump, rest, OPT_ISSET(ops, b'm') as i32, map, what)
    } else {
        build_dump(nam, &dump, rest, OPT_ISSET(ops, b'U') as i32, map, flags)
    }
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
    crate::ported::lex::zshlex();
}

/// `COND_SEP()` macro from `Src/parse.c:2433`. True when the current
/// token is a separator usable inside `[[ … ]]` (newline / semi /
/// `&`). C uses it to skip optional whitespace between cond terms.
#[inline]
pub fn COND_SEP() -> bool {
    matches!(tok(), NEWLIN | SEMI | AMPER)
}

/// Port of `copy_ecstr(Eccstr s, char *p)` from `Src/parse.c:537`.
/// Walks the in-build string-eccstr tree and writes each entry to
/// `p[s->aoffs..]`. The Rust port mirrors via the
/// `ECSTRS_REVERSE` HashMap (eccstr-tree replacement) and writes
/// into a `Vec<u8>` slice.
pub fn copy_ecstr(table: &std::collections::HashMap<u32, String>, p: &mut [u8]) { // c:537
    for (&offs, s) in table.iter() {
        let off = offs as usize;
        let bytes = s.as_bytes();
        let need = off + bytes.len() + 1;
        if need > p.len() {
            continue;
        }
        p[off..off + bytes.len()].copy_from_slice(bytes);
        p[off + bytes.len()] = 0;
    }
}

/// Port of `bld_eprog(int heap)` from `Src/parse.c:547`. Finalizes
/// the in-build `ECBUF`/`ECSTRS`/`ECNPATS` state into an `Eprog`.
/// Resets the build state so a new parse can start.
pub fn bld_eprog(heap: bool) -> crate::ported::zsh_h::eprog {                 // c:547

    // c:555 — emit WC_END opcode. `WCB_END` is `WC_END_DEFAULT` (0).
    ecadd(0);

    let ecused  = ECUSED.with(|c| c.get())  as usize;
    let ecnpats = ECNPATS.with(|c| c.get()) as usize;
    let ecsoffs = ECSOFFS.with(|c| c.get()) as usize;

    let prog_bytes = ecused * 4;                                              // c:559
    let len = (ecnpats * 4) + prog_bytes + ecsoffs;

    // Snapshot the wordcode buffer + string table.
    let prog_words: Vec<u32> = ECBUF.with(|c| c.borrow()[..ecused].to_vec());
    let mut strs_bytes = vec![0u8; ecsoffs];
    ECSTRS_REVERSE.with(|c| copy_ecstr(&c.borrow(), &mut strs_bytes));

    let ret = eprog {
        flags: if heap { EF_HEAP } else { EF_REAL },                          // c:570
        len: len as i32,                                                      // c:559
        npats: ecnpats as i32,                                                // c:561
        nref: if heap { -1 } else { 1 },                                      // c:562
        pats: Vec::new(),                                                     // c:563 dummy_patprog
        prog: prog_words,                                                     // c:565
        strs: Some(String::from_utf8_lossy(&strs_bytes).into_owned()),        // c:566
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

    ret
}

/// Port of `parse_list(void)` from `Src/parse.c:697`. C-shape entry
/// point: drives `par_list` and finalizes via `bld_eprog`. Returns
/// `None` on syntax error.
pub fn parse_list() -> Option<crate::ported::zsh_h::eprog> {                  // c:697
    set_tok(ENDINPUT);
    init_parse();
    zshlex();
    let _ = par_list();
    if tok() != ENDINPUT {
        clear_hdocs();
        set_tok(LEXERR);
        yyerror("syntax error");
        return None;
    }
    Some(bld_eprog(true))
}

/// Port of `parse_cond(void)` from `Src/parse.c:722`. Only used by
/// `bin_test`/`bin_bracket` for `/bin/test`/`[` compat — the
/// `condlex` global must already point at `testlex` before entry.
pub fn parse_cond() -> Option<crate::ported::zsh_h::eprog> {                  // c:722
    init_parse();
    if par_cond().is_none() {
        clear_hdocs();
        return None;
    }
    Some(bld_eprog(true))
}

/// Port of `par_sublist2(int *cmplx)` from `Src/parse.c:869`.
/// Secondary-sublist arm: handles the `COPROC`/`BANG` prefix
/// in front of a pline. Returns the WC_SUBLIST flag word added.
pub fn par_sublist2(cmplx: &mut i32) -> Option<i32> {                         // c:869
    let mut f = 0i32;
    if tok() == COPROC {
        *cmplx = 1;
        f |= crate::ported::zsh_h::WC_SUBLIST_COPROC as i32;
        zshlex();
    } else if tok() == BANG_TOK {
        *cmplx = 1;
        f |= crate::ported::zsh_h::WC_SUBLIST_NOT as i32;
        zshlex();
    }
    // c:884 — `if (!par_pline(cmplx) && !f) return -1;`
    let pl = par_pline();
    if pl.is_none() && f == 0 {
        return None;
    }
    Some(f)
}

/// Port of `par_dinbrack(void)` from `Src/parse.c:1810`. Body
/// parser inside `[[ ... ]]` — calls `par_cond` to emit the
/// condition wordcode then advances past `]]`.
pub fn par_dinbrack() -> Option<()> {                                         // c:1810
    set_incond(1);                                                            // c:1814
    set_incmdpos(false);                                                      // c:1815
    zshlex();                                                                 // c:1816
    let _ = par_cond();                                                       // c:1817
    if tok() != DOUTBRACK {                                                   // c:1818
        yyerror("missing ]]");
        return None;
    }
    set_incond(0);                                                            // c:1820
    set_incmdpos(true);                                                       // c:1821
    zshlex();                                                                 // c:1822
    Some(())
}

/// Port of `par_cond_1(void)` from `Src/parse.c:2434`. Parses one
/// `||`-separated cond expression. Emits `WCB_COND(COND_AND, …)`
/// when an `&&` is found and recurses.
pub fn par_cond_1() -> i32 {                                                  // c:2434

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
pub fn par_cond_2() -> i32 {                                                  // c:2476
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
        while COND_SEP() { condlex(); }
        let r = par_cond();
        while COND_SEP() { condlex(); }
        if tok() != OUTPAR_TOK {
            yyerror("missing )");
            return 0;
        }
        condlex();
        return r.map_or(0, |_| 1);
    }
    let s1 = tokstr().unwrap_or_default();
    let dble = s1.starts_with('-')
        && s1.len() == 2
        && "abcdefghknoprstuvwxzLONGS".contains(s1.chars().nth(1).unwrap_or('?'));
    if tok() != STRING_LEX {
        if !s1.is_empty() && tok() != LEXERR && (!dble || n_testargs != 0) {
            // c:2557 — `[[ STRING ]]` re-interpreted as `[[ -n STRING ]]`.
            condlex(); while COND_SEP() { condlex(); }
            return par_cond_double("-n", &s1);
        }
        yyerror("condition expected");
        return 0;
    }
    condlex();
    while COND_SEP() { condlex(); }
    if tok() == INANG_TOK || tok() == OUTANG_TOK {
        // c:2576 — `<` / `>` string compare.
        let xtok = tok();
        condlex();
        while COND_SEP() { condlex(); }
        if tok() != STRING_LEX {
            yyerror("string expected");
            return 0;
        }
        let s3 = tokstr().unwrap_or_default();
        condlex();
        while COND_SEP() { condlex(); }
        let op = if xtok == INANG_TOK { COND_STRLT } else { COND_STRGTR };
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
        yyerror("syntax error");
        return 0;
    }
    let s2 = tokstr().unwrap_or_default();
    crate::ported::lex::set_incond(crate::ported::lex::incond() + 1);
    condlex(); while COND_SEP() { condlex(); }
    crate::ported::lex::set_incond(crate::ported::lex::incond() - 1);
    if tok() == STRING_LEX && !dble {
        let s3 = tokstr().unwrap_or_default();
        condlex(); while COND_SEP() { condlex(); }
        if tok() == STRING_LEX {
            // c:2615 — n-ary `[ A op B C D ... ]`.
            let mut l: Vec<String> = vec![s2, s3];
            while tok() == STRING_LEX {
                l.push(tokstr().unwrap_or_default());
                condlex(); while COND_SEP() { condlex(); }
            }
            return par_cond_multi(&s1, &l);
        }
        return par_cond_triple(&s1, &s2, &s3);
    }
    par_cond_double(&s1, &s2)
}

/// Port of `par_cond_double(char *a, char *b)` from `Src/parse.c:2626`.
/// Emits wordcode for unary cond `[ -X b ]` or modular `[ -mod b ]`.
pub fn par_cond_double(a: &str, b: &str) -> i32 {                             // c:2626
    if !a.starts_with('-') || a.len() < 2 {
        crate::ported::utils::zerr(&format!("parse error: condition expected: {}", a));
        return 1;
    }
    let unary_set = "abcdefgknoprstuvwxzhLONGS";
    if a.len() == 2 && unary_set.contains(a.chars().nth(1).unwrap_or('?')) {
        ecadd(WCB_COND(a.as_bytes()[1] as u32, 0));
        ecstr(b);
    } else {
        ecadd(WCB_COND(COND_MOD as u32, 1));
        ecstr(a);
        ecstr(b);
    }
    1
}

/// Port of `par_cond_triple(char *a, char *b, char *c)` from
/// `Src/parse.c:2659`. Emits wordcode for the binary forms
/// `[ A op B ]` — `=` / `==` / `!=` / `<` / `>` / `=~` / `-X`.
pub fn par_cond_triple(a: &str, b: &str, c: &str) -> i32 {                    // c:2659


    let bb = b.as_bytes();
    if (bb == b"=" || bb == b"==") && bb.len() <= 2 {
        let dbl = bb.len() == 2;
        let op = if dbl { COND_STRDEQ } else { COND_STREQ };
        ecadd(WCB_COND(op as u32, 0));
        ecstr(a);
        ecstr(c);
        let np = ECNPATS.with(|cc| { let v = cc.get(); cc.set(v + 1); v }) as u32;
        ecadd(np);
        return 1;
    }
    if (bb == b">" || bb == b"<") && bb.len() == 1 {
        let op = if bb[0] == b'>' { COND_STRGTR } else { COND_STRLT };
        ecadd(WCB_COND(op as u32, 0));
        ecstr(a);
        ecstr(c);
        let np = ECNPATS.with(|cc| { let v = cc.get(); cc.set(v + 1); v }) as u32;
        ecadd(np);
        return 1;
    }
    if bb == b"!=" {
        ecadd(WCB_COND(COND_STRNEQ as u32, 0));
        ecstr(a);
        ecstr(c);
        let np = ECNPATS.with(|cc| { let v = cc.get(); cc.set(v + 1); v }) as u32;
        ecadd(np);
        return 1;
    }
    if bb == b"=~" {
        ecadd(WCB_COND(COND_REGEX as u32, 0));
        ecstr(a);
        ecstr(c);
        return 1;
    }
    if b.starts_with('-') {
        let t = get_cond_num(&b[1..]);
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
    if a.starts_with('-') && a.len() > 1 {
        ecadd(WCB_COND(COND_MOD as u32, 2));
        ecstr(a);
        ecstr(b);
        ecstr(c);
        return 1;
    }
    crate::ported::utils::zerr(&format!("condition expected: {}", b));
    1
}

/// Port of `par_cond_multi(char *a, LinkList l)` from `Src/parse.c:2716`.
/// Emits wordcode for `[ -OP A B C … ]` n-ary cond (alternation).
pub fn par_cond_multi(a: &str, l: &[String]) -> i32 {                         // c:2716
    if !a.starts_with('-') || a.len() < 2 {
        crate::ported::utils::zerr(&format!("condition expected: {}", a));
        return 1;
    }
    ecadd(WCB_COND(COND_MOD as u32, l.len() as u32));
    ecstr(a);
    for item in l {
        ecstr(item);
    }
    1
}

/// Port of `cur_add_func(char *nam, Shfunc shf, LinkList names, LinkList progs, int *hlen, int *tlen, int what)`
/// from `Src/parse.c:3489`. Adds a shfunc to the in-build dump
/// progs+names lists. Stub: `Eprog` for the function body isn't
/// yet wired through `shfunc.funcdef` to be serializable here.
pub fn cur_add_func(nam: &str,                                                // c:3489
                    shf_name: &str,
                    shf_flags: i32,
                    names: &mut Vec<String>,
                    progs: &mut Vec<wcfunc>,
                    hlen: &mut i32,
                    tlen: &mut i32,
                    what: i32) -> i32 {

    let is_undef = (shf_flags as u32 & PM_UNDEFINED) != 0;
    if is_undef {
        if (what & 2) == 0 {                                                  // c:3498
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
    // c:3517 — would `dupeprog(shf->funcdef)`. Stub: empty body.
    let wcf = wcfunc {
        name: shf_name.to_string(),
        flags: FDHF_ZSHLOAD,
        body: Vec::new(),
    };
    progs.push(wcf);
    names.push(shf_name.to_string());

    // c:3526 — bump hlen / tlen.
    let name_words = (shf_name.len() as i32 + 4) / 4;
    *hlen += (FDHEAD_WORDS as i32) + name_words;
    *tlen += 0; // body is empty in stub; real path adds prog->len in words.

    0
}

/// Port of `write_dump(int dfd, LinkList progs, int map, int hlen, int tlen)`
/// from `Src/parse.c:3334`. Writes the prelude + header records +
/// body wordcode bytes to the dump file descriptor.
///
/// Two passes: first native-byte-order (`FD_MAGIC`), then opposite-
/// byte-order (`FD_OMAGIC`) so big-endian readers can mmap the
/// same file. Bodies are byte-swapped via `fdswap` on the second pass.
pub fn write_dump(dfd: &mut std::fs::File,                                    // c:3334
                  progs: &[wcfunc],
                  mut map: i32,
                  hlen: i32,
                  tlen: i32) -> std::io::Result<()> {
    if map == 1 && (tlen as usize) >= FD_MINMAP {                             // c:3344
        map = 1;
    } else if map == 1 {
        map = 0;
    }

    let mut other = 0u32;                                                     // c:3338
    let ohlen = hlen;
    let mut cur_hlen = hlen;

    loop {
        cur_hlen = ohlen;
        // c:3347 — build the prelude.
        let mut pre = vec![0u32; FD_PRELEN];
        pre[0] = if other != 0 { FD_OMAGIC } else { FD_MAGIC };               // c:3350
        let flags = (if map != 0 { FDF_MAP } else { 0 }) | other;
        fdsetflags(&mut pre, flags as u8);                                    // c:3351
        fdsetother(&mut pre, tlen as u32);                                    // c:3352
        // c:3353 — copy ZSH_VERSION C-string into pre[2..].
        let ver = b"5.9";
        for (i, &b) in ver.iter().enumerate() {
            let word = 2 + i / 4;
            let shift = (i % 4) * 8;
            pre[word] |= (b as u32) << shift;
        }
        // Write prelude.
        for w in &pre {
            dfd.write_all(&w.to_le_bytes())?;
        }
        // c:3356 — per-fn header records.
        for wcf in progs {
            let n = &wcf.name;
            let prog = &wcf.body;
            let mut head = fdhead {
                start: cur_hlen as u32,                                       // c:3360
                len: (prog.len() * 4) as u32,                                 // c:3363
                npats: 0,                                                     // c:3364 (npats not tracked yet)
                strs: 0,                                                      // c:3365
                hlen: ((FDHEAD_WORDS as u32) + ((n.len() as u32 + 4) / 4)),   // c:3366
                flags: 0,
            };
            cur_hlen += prog.len() as i32;                                    // c:3361
            // c:3368 — name tail offset from path basename.
            let tail = n.rfind('/').map(|p| p + 1).unwrap_or(0);
            head.flags = fdhbldflags(wcf.flags, tail as u32);                 // c:3372
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
            // c:3376 — write the name + NUL + pad-to-4.
            dfd.write_all(n.as_bytes())?;
            dfd.write_all(&[0u8])?;
            let pad = (4 - ((n.len() + 1) & 3)) & 3;
            if pad > 0 {
                dfd.write_all(&vec![0u8; pad])?;
            }
        }
        // c:3381 — per-fn body words.
        for wcf in progs {
            let mut body = wcf.body.clone();
            if other != 0 {
                fdswap(&mut body);
            }
            for w in &body {
                dfd.write_all(&w.to_le_bytes())?;
            }
        }
        if other != 0 {                                                       // c:3389
            break;
        }
        other = FDF_OTHER;                                                    // c:3391
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::utils::{errflag, ERRFLAG_ERROR};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    /// Test helper. Mirrors zsh's `errflag` save/clear/check pattern
    /// around a parse — see `Src/init.c:loop` which clears errflag
    /// before parse_event() and tests it after. Returns `Err` if the
    /// parse set `ERRFLAG_ERROR`; otherwise `Ok(program)`.
    fn parse(input: &str) -> Result<ZshProgram, String> {
        let saved = errflag.load(Ordering::Relaxed);
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        crate::ported::parse::parse_init(input);
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
        let prog = parse("ls | grep foo | wc -l").unwrap();
        assert_eq!(prog.lists.len(), 1);

        let pipe = &prog.lists[0].sublist.pipe;
        assert!(pipe.next.is_some());

        let pipe2 = pipe.next.as_ref().unwrap();
        assert!(pipe2.next.is_some());
    }

    #[test]
    fn test_and_or() {
        let prog = parse("cmd1 && cmd2 || cmd3").unwrap();
        let sublist = &prog.lists[0].sublist;

        assert!(sublist.next.is_some());
        let (op, _) = sublist.next.as_ref().unwrap();
        assert_eq!(*op, SublistOp::And);
    }

    #[test]
    fn test_if_then() {
        let prog = parse("if test -f foo; then echo yes; fi").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::If(_) => {}
            _ => panic!("expected if command"),
        }
    }

    #[test]
    fn test_for_loop() {
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

    #[test]
    #[ignore] // Uses threads that can't be killed on timeout; use integration test instead
    fn test_parse_zsh_stdlib_functions() {

        let functions_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_data/zsh_functions");
        if !functions_dir.exists() {
            eprintln!(
                "Skipping test: zsh_functions directory not found at {:?}",
                functions_dir
            );
            return;
        }

        let mut total = 0;
        let mut passed = 0;
        let mut failed_files = Vec::new();
        let mut timeout_files = Vec::new();

        if let Ok(entries) = fs::read_dir(&functions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                total += 1;
                let file_path = path.display().to_string();
                let content = match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        failed_files.push((file_path, format!("read error: {}", e)));
                        continue;
                    }
                };

                // Parse with timeout
                let content_clone = content.clone();
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
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
                    }
                }
            }
        }

        eprintln!("\n=== Zsh Stdlib Functions Parse Results ===");
        eprintln!("Passed: {}/{}", passed, total);

        if !timeout_files.is_empty() {
            eprintln!("\nTimeout files (>2s): {}", timeout_files.len());
            for file in timeout_files.iter().take(10) {
                eprintln!("  {}", file);
            }
            if timeout_files.len() > 10 {
                eprintln!("  ... and {} more", timeout_files.len() - 10);
            }
        }

        if !failed_files.is_empty() {
            eprintln!("\nFailed files: {}", failed_files.len());
            for (file, err) in failed_files.iter().take(20) {
                let filename = Path::new(file)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                eprintln!("  {} - {}", filename, err);
            }
            if failed_files.len() > 20 {
                eprintln!("  ... and {} more", failed_files.len() - 20);
            }
        }

        let pass_rate = if total > 0 {
            (passed as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        eprintln!("Pass rate: {:.1}%", pass_rate);

        // Require at least 50% pass rate
        assert!(pass_rate >= 50.0, "Pass rate too low: {:.1}%", pass_rate);
    }
}
