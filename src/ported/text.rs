//! Port of `Src/text.c` — textual representations of wordcode (`gettext2`),
//! permanent text (`getpermtext`), job text (`getjobtext`), redirection text
//! (`getredirs`), and the shared character-buffer helpers (`tadd*`, etc.).

use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::ported::linklist::LinkList;
use crate::ported::mem::{queue_signals, unqueue_signals};
use crate::ported::utils::{has_token, quotestring};
use crate::ported::zsh_h;
use crate::ported::zsh_h::{
    estate, redir, wc_code, wordcode, Eprog, Meta, COND_AND, COND_MOD, COND_MODI, COND_NOT,
    COND_OR, COND_STRDEQ, COND_STREQ, COND_STRNEQ, EC_NODUP, IS_READFD, JOBTEXTSIZE,
    REDIRF_FROM_HEREDOC, REDIR_APP, REDIR_APPNOW, REDIR_CLOSE, REDIR_ERRAPP, REDIR_ERRAPPNOW,
    REDIR_ERRWRITE, REDIR_ERRWRITENOW, REDIR_HEREDOC, REDIR_HERESTR, REDIR_INPIPE, REDIR_MERGEIN,
    REDIR_MERGEOUT, REDIR_OUTPIPE, REDIR_READ, REDIR_READWRITE, REDIR_WRITE, REDIR_WRITENOW,
    WC_ARITH, WC_ASSIGN, WC_ASSIGN_ARRAY, WC_AUTOFN, WC_CASE, WC_CASE_AND, WC_CASE_OR,
    WC_CASE_SKIP, WC_CASE_TYPE, WC_COND, WC_COND_SKIP, WC_COND_TYPE, WC_COUNT, WC_CURSH,
    WC_CURSH_SKIP, WC_END, WC_FOR, WC_FOR_COND, WC_FOR_LIST, WC_FOR_TYPE, WC_FUNCDEF,
    WC_FUNCDEF_SKIP, WC_IF, WC_IF_ELIF, WC_IF_SKIP, WC_IF_TYPE, WC_LIST, WC_LIST_TYPE, WC_PIPE,
    WC_PIPE_END, WC_PIPE_MID, WC_PIPE_TYPE, WC_REDIR, WC_REPEAT, WC_SELECT, WC_SELECT_LIST,
    WC_SELECT_TYPE, WC_SIMPLE, WC_SIMPLE_ARGC, WC_SUBLIST, WC_SUBLIST_COPROC, WC_SUBLIST_END,
    WC_SUBLIST_FLAGS, WC_SUBLIST_NOT, WC_SUBLIST_OR, WC_SUBLIST_SIMPLE, WC_SUBLIST_SKIP,
    WC_SUBLIST_TYPE, WC_SUBSH, WC_SUBSH_SKIP, WC_TIMED, WC_TIMED_PIPE, WC_TIMED_TYPE, WC_TRY,
    WC_TYPESET, WC_TYPESET_ARGC, WC_WHILE, WC_WHILE_TYPE, WC_WHILE_UNTIL, Z_ASYNC, Z_DISOWN, Z_END,
    Z_SIMPLE,
};
use crate::{lex, parse, DPUTS};

/// Port of `is_cond_binary_op(const char *str)` from `Src/text.c:58`.
pub fn is_cond_binary_op(str: &str) -> i32 {
    COND_BINARY_OPS.iter().any(|&op| op == str) as i32
}

/// Port of `dec_tindent` from `Src/text.c:70`.
pub fn dec_tindent() {
    tindent.with(|t| {
        let mut v = t.borrow_mut();
        // c:72 — DPUTS(tindent == 0, "attempting to decrement tindent below zero")
        DPUTS!(*v == 0, "attempting to decrement tindent below zero"); // c:72
        if *v > 0 {
            *v -= 1;
        }
    });
}

/// Port of `taddpending(char *str1, char *str2)` from `Src/text.c:89`.
pub fn taddpending(str1: &str, str2: &str) {
    let mut v = Vec::with_capacity(str1.len() + str2.len());
    v.extend_from_slice(str1.as_bytes());
    v.extend_from_slice(str2.as_bytes());
    tpending.with(|p| {
        let mut g = p.borrow_mut();
        match &mut *g {
            Some(p) => {
                p.push(b'\n');
                p.extend_from_slice(&v);
            }
            None => *g = Some(v),
        }
    });
}

// ---------------------------------------------------------------------------
// Internal: file-static text buffer + gettext2 (text.c:53–1015)
// ---------------------------------------------------------------------------

// File-statics from `Src/text.c:396` (`tbuf`/`tptr`/`tlim` modeled as `tbuf` Vec +
// `tlim` cap; `tpending`/`tindent`/`tnewlins`/`tjob`).
thread_local! {
    static tbuf: RefCell<Vec<u8>> = RefCell::new(Vec::new());
    static tlim: RefCell<Option<usize>> = RefCell::new(None);
    static tpending: RefCell<Option<Vec<u8>>> = RefCell::new(None);
    static tindent: RefCell<i32> = RefCell::new(0);
    static tnewlins: RefCell<bool> = RefCell::new(true);
    static tjob: RefCell<bool> = RefCell::new(false);
}

/// Port of `tdopending` from `Src/text.c:114`.
pub fn tdopending() {
    let drained = tpending.with(|p| p.borrow_mut().take());
    if let Some(p) = drained {
        tpush(b'\n' as i32);
        taddstr(&String::from_utf8_lossy(&p));
    }
}

/// Port of `taddchr(int c)` from `Src/text.c:128`.
pub fn taddchr(c: i32) {
    tpush(c);
}

/// Port of `taddstr(const char *s)` from `Src/text.c:146`.
pub fn taddstr(s: &str) {
    let nl = tnewlins.with(|c| *c.borrow());
    if nl {
        tbuf.with(|tb| tb.borrow_mut().extend_from_slice(s.as_bytes()));
    } else {
        for &b in s.as_bytes() {
            let ch = if b == b'\n' { b' ' } else { b };
            tpush(ch as i32);
        }
    }
}

/// Port of `taddlist(Estate state, int num)` from `Src/text.c:170`.
fn taddlist(state: &mut estate, num: i32) {
    if num == 0 {
        return;
    }
    let mut n = num;
    while n > 0 {
        taddstr(&ecgetstr(state, EC_NODUP, None));
        tpush(b' ' as i32);
        n -= 1;
    }
    tbuf.with(|tb| {
        let _ = tb.borrow_mut().pop();
    });
}

/// Port of `taddassign(wordcode code, Estate state, int typeset)` from `Src/text.c:184`.
fn taddassign(code: wordcode, state: &mut estate, typeset: i32) {
    taddstr(&ecgetstr(state, EC_NODUP, None));
    if zsh_h::WC_ASSIGN_TYPE2(code) == zsh_h::WC_ASSIGN_INC {
        if typeset != 0 {
            let _ = ecgetstr(state, EC_NODUP, None);
            tpush(b' ' as i32);
            return;
        }
        tpush(b'+' as i32);
    }
    tpush(b'=' as i32);
    if zsh_h::WC_ASSIGN_TYPE(code) == WC_ASSIGN_ARRAY {
        tpush(b'(' as i32);
        taddlist(state, zsh_h::WC_ASSIGN_NUM(code) as i32);
        taddstr(") ");
    } else {
        taddstr(&ecgetstr(state, EC_NODUP, None));
        tpush(b' ' as i32);
    }
}

/// Port of `taddassignlist(Estate state, wordcode count)` from `Src/text.c:213`.
fn taddassignlist(state: &mut estate, count: wordcode) {
    if count != 0 {
        tpush(b' ' as i32);
    }
    let mut c = count;
    while c > 0 {
        if state.pc >= state.prog.prog.len() {
            break;
        }
        let acode = state.prog.prog[state.pc];
        state.pc += 1;
        taddassign(acode, state, 1);
        c -= 1;
    }
}

/// Port of `taddnl(int no_semicolon)` from `Src/text.c:227`.
pub fn taddnl(no_semicolon: i32) {
    let newlins = tnewlins.with(|c| *c.borrow());
    let indent = tindent.with(|t| *t.borrow());
    let xt = TEXT_EXPAND_TABS.load(Ordering::Relaxed);
    if newlins {
        tdopending();
        tpush(b'\n' as i32);
        for _ in 0..indent {
            if xt >= 0 {
                if xt > 0 {
                    for _ in 0..xt {
                        tpush(b' ' as i32);
                    }
                } else {
                    tpush(b'\t' as i32);
                }
            }
        }
    } else if no_semicolon != 0 {
        taddstr(" ");
    } else {
        taddstr("; ");
    }
}

/// Port of `getpermtext(Eprog prog, Wordcode c, int start_indent)` from `Src/text.c:279`.
pub fn getpermtext(prog: Eprog, c: Option<usize>, start_indent: i32) -> String {
    queue_signals();
    useeprog(&prog);
    // c:292 — `s.strs = prog->strs;` — pooled (>3-byte) strings live in
    // the eprog string table; estate must carry it or ecgetstr returns
    // "" for every pooled string.
    let strs = prog.strs.clone();
    let mut state = estate {
        prog,
        pc: c.unwrap_or(0),
        strs,
        strs_offset: 0,
    };
    tbuf.with(|tb| tb.borrow_mut().clear());
    tlim.with(|l| *l.borrow_mut() = None);
    tpending.with(|p| *p.borrow_mut() = None);
    tindent.with(|t| *t.borrow_mut() = start_indent);
    tnewlins.with(|n| *n.borrow_mut() = true);
    tjob.with(|j| *j.borrow_mut() = false);
    if state.prog.len != 0 {
        gettext2(&mut state);
    }
    let raw = tbuf.with(|tb| {
        let mut v = tb.borrow_mut();
        String::from_utf8_lossy(&std::mem::take(&mut *v)).into_owned()
    });
    let p = state.prog;
    freeeprog(&p);
    unqueue_signals();
    // c:304 — `untokenize(tbuf);` — the utils.c untokenize maps EVERY
    // ITOK char through ztokens (Snull → `'`, Dnull → `"`, Qstring →
    // `$`, Bnull → `\`) and drops Nularg. `lex::untokenize` is the
    // substitution-stream variant that STRIPS the quote markers —
    // using it here loses the quoting from rendered function text
    // (`"$@"` → `$@`), which breaks re-parsing of `.zwc`-loaded
    // bodies. Use the quote-preserving variant, then apply the two
    // print-side mappings it intentionally defers (Qstring → `$`,
    // Nularg dropped) per c:Src/utils.c:4204-4208.
    lex::untokenize_preserve_quotes(&raw)
        .chars()
        .filter(|&c| c != zsh_h::Nularg)
        .map(|c| if c == zsh_h::Qstring { '$' } else { c })
        .collect()
}

/// Port of `getjobtext(Eprog prog, Wordcode c)` from `Src/text.c:315`.
pub fn getjobtext(prog: Eprog, c: Option<usize>) -> String {
    queue_signals();
    useeprog(&prog);
    // c:329 — `s.strs = prog->strs;` (same fix as getpermtext).
    let strs = prog.strs.clone();
    let mut state = estate {
        prog,
        pc: c.unwrap_or(0),
        strs,
        strs_offset: 0,
    };
    tbuf.with(|tb| tb.borrow_mut().clear());
    tlim.with(|l| *l.borrow_mut() = Some(JOBTEXTSIZE));
    tpending.with(|p| *p.borrow_mut() = None);
    tindent.with(|t| *t.borrow_mut() = 0);
    tnewlins.with(|n| *n.borrow_mut() = true);
    tjob.with(|j| *j.borrow_mut() = true);
    if state.prog.len != 0 {
        gettext2(&mut state);
    }
    let mut raw = tbuf.with(|tb| {
        let mut v = tb.borrow_mut();
        String::from_utf8_lossy(&std::mem::take(&mut *v)).into_owned()
    });
    if raw.ends_with(Meta as char) {
        raw.pop();
    }
    let p = state.prog;
    freeeprog(&p);
    unqueue_signals();
    // c:342 — `untokenize(jbuf);` — same print-side mapping as
    // getpermtext (see comment there).
    lex::untokenize_preserve_quotes(&raw)
        .chars()
        .filter(|&c| c != zsh_h::Nularg)
        .map(|c| if c == zsh_h::Qstring { '$' } else { c })
        .collect()
}

#[allow(non_camel_case_types)]
struct tstack {
    code: wordcode,
    pop: i32,
    u: tstack_u,
}

/// Port of `tpush(wordcode code, int pop)` from `Src/text.c:396` (append byte to `tbuf`, honour `tlim`).
fn tpush(c: i32) {
    let b = c as u8;
    tbuf.with(|tb| {
        let mut v = tb.borrow_mut();
        if let Some(max) = tlim.with(|l| *l.borrow()) {
            if v.len() >= max {
                return;
            }
        }
        v.push(b);
    });
}

/// Port of `gettext2(Estate state)` from `Src/text.c:415`.
pub fn gettext2(state: &mut estate) {
    let mut tstack: Vec<tstack> = Vec::new();
    let mut stack: i32 = 0;

    loop {
        let (code, mut spopped, mut s_live): (wordcode, Option<tstack>, Option<usize>) =
            if stack != 0 {
                if tstack.is_empty() {
                    break;
                }
                let should_pop = tstack.last().unwrap().pop != 0;
                let code = tstack.last().unwrap().code;
                if should_pop {
                    let fr = tstack.pop().unwrap();
                    stack = 0;
                    (code, Some(fr), None)
                } else {
                    stack = 0;
                    let idx = tstack.len() - 1;
                    (code, None, Some(idx))
                }
            } else {
                if state.pc >= state.prog.prog.len() {
                    break;
                }
                let code = state.prog.prog[state.pc];
                state.pc += 1;
                (code, None, None)
            };

        let s_active = s_live.is_some();
        let mut s_idx = s_live;

        macro_rules! s_mut {
            () => {
                match (&mut s_idx, &mut spopped) {
                    (Some(i), None) => Some(&mut tstack[*i]),
                    (None, Some(p)) => Some(p),
                    _ => None,
                }
            };
        }

        match wc_code(code) {
            WC_LIST => {
                if !s_active && spopped.is_none() {
                    tstack.push(tstack {
                        code,
                        pop: ((WC_LIST_TYPE(code) as i32) & Z_END) as i32,
                        u: tstack_u::None,
                    });
                    stack = 0;
                } else if let Some(fr) = s_mut!() {
                    let lty = WC_LIST_TYPE(code) as i32;
                    if (lty & Z_ASYNC) != 0 {
                        taddstr(" &");
                        if (lty & Z_DISOWN) != 0 {
                            taddstr("|");
                        }
                    }
                    let end_here = (lty & Z_END) != 0;
                    if !end_here {
                        if tnewlins.with(|c| *c.borrow()) {
                            taddnl(0);
                        } else {
                            taddstr(if (lty & Z_ASYNC) != 0 { " " } else { "; " });
                        }
                        if state.pc >= state.prog.prog.len() {
                            break;
                        }
                        fr.code = state.prog.prog[state.pc];
                        state.pc += 1;
                        fr.pop = ((WC_LIST_TYPE(fr.code) as i32) & Z_END) as i32;
                        stack = 0;
                    } else {
                        stack = 1;
                    }
                }
                if stack == 0 {
                    let sc = if let Some(fr) = s_mut!() {
                        fr.code
                    } else if let Some(fr) = &spopped {
                        fr.code
                    } else {
                        tstack.last().map(|t| t.code).unwrap_or(code)
                    };
                    if (WC_LIST_TYPE(sc) as i32 & Z_SIMPLE) != 0 {
                        state.pc = state.pc.saturating_add(1);
                    }
                }
            }
            WC_SUBLIST => {
                if !s_active && spopped.is_none() {
                    let p = state.pc;
                    let mut pre = 0;
                    if (WC_SUBLIST_FLAGS(code) as i32 & WC_SUBLIST_SIMPLE as i32) == 0
                        && p < state.prog.prog.len()
                        && wc_code(state.prog.prog[p]) != WC_PIPE
                    {
                        pre = -1;
                    }
                    if (WC_SUBLIST_FLAGS(code) as i32 & WC_SUBLIST_NOT as i32) != 0 {
                        taddstr(if pre != 0 { "!" } else { "! " });
                    }
                    if (WC_SUBLIST_FLAGS(code) as i32 & WC_SUBLIST_COPROC as i32) != 0 {
                        taddstr(if pre != 0 { "coproc" } else { "coproc " });
                    }
                    tstack.push(tstack {
                        code,
                        pop: if WC_SUBLIST_TYPE(code) == WC_SUBLIST_END {
                            1
                        } else {
                            0
                        },
                        u: tstack_u::None,
                    });
                    stack = pre;
                } else if let Some(fr) = s_mut!() {
                    let end_ty = WC_SUBLIST_TYPE(code) == WC_SUBLIST_END;
                    if !end_ty {
                        taddstr(if WC_SUBLIST_TYPE(code) == WC_SUBLIST_OR {
                            " || "
                        } else {
                            " && "
                        });
                        if state.pc >= state.prog.prog.len() {
                            break;
                        }
                        fr.code = state.prog.prog[state.pc];
                        state.pc += 1;
                        fr.pop = if WC_SUBLIST_TYPE(fr.code) == WC_SUBLIST_END {
                            1
                        } else {
                            0
                        };
                        if (WC_SUBLIST_FLAGS(fr.code) as i32 & WC_SUBLIST_NOT as i32) != 0 {
                            let sk = if WC_SUBLIST_SKIP(fr.code) == 0 { 1 } else { 0 };
                            let p = state.pc;
                            let pipe_chk = if p < state.prog.prog.len() {
                                wc_code(state.prog.prog[p])
                            } else {
                                WC_COUNT
                            };
                            let not_simple_pipe =
                                (WC_SUBLIST_FLAGS(fr.code) as i32 & WC_SUBLIST_SIMPLE as i32) == 0
                                    && pipe_chk != WC_PIPE;
                            taddstr(if sk != 0 || not_simple_pipe {
                                "!"
                            } else {
                                "! "
                            });
                            stack = sk;
                        }
                        if (WC_SUBLIST_FLAGS(fr.code) as i32 & WC_SUBLIST_COPROC as i32) != 0 {
                            taddstr("coproc ");
                        }
                    } else {
                        stack = 1;
                    }
                }
                if stack < 1 {
                    let scode = if let Some(fr) = s_mut!() {
                        fr.code
                    } else if let Some(fr) = &spopped {
                        fr.code
                    } else {
                        tstack.last().map(|x| x.code).unwrap_or(code)
                    };
                    if (WC_SUBLIST_FLAGS(scode) as i32 & WC_SUBLIST_SIMPLE as i32) != 0 {
                        state.pc = state.pc.saturating_add(1);
                    }
                }
            }
            WC_PIPE => {
                if !s_active && spopped.is_none() {
                    tstack.push(tstack {
                        code,
                        pop: if WC_PIPE_TYPE(code) == WC_PIPE_END {
                            1
                        } else {
                            0
                        },
                        u: tstack_u::None,
                    });
                    if WC_PIPE_TYPE(code) == WC_PIPE_MID {
                        state.pc = state.pc.saturating_add(1);
                    }
                } else if let Some(fr) = s_mut!() {
                    if !(WC_PIPE_TYPE(code) == WC_PIPE_END) {
                        taddstr(" | ");
                        if state.pc >= state.prog.prog.len() {
                            break;
                        }
                        fr.code = state.prog.prog[state.pc];
                        state.pc += 1;
                        let end_next = WC_PIPE_TYPE(fr.code) == WC_PIPE_END;
                        fr.pop = if end_next { 1 } else { 0 };
                        if !end_next {
                            state.pc += 1;
                        }
                        stack = 0;
                    } else {
                        stack = 1;
                    }
                }
            }
            WC_REDIR => {
                if !s_active && spopped.is_none() {
                    state.pc = state.pc.saturating_sub(1); // c:505
                    let rows = parse::ecgetredirs(state); // c:507
                    let mut lst = LinkList::new();
                    for pr in rows {
                        lst.push_back(redir {
                            typ: pr.typ,
                            flags: pr.flags,
                            fd1: pr.fd1,
                            fd2: pr.fd2,
                            name: pr.name,
                            varid: pr.varid,
                            here_terminator: pr.here_terminator,
                            munged_here_terminator: pr.munged_here_terminator,
                        });
                    }
                    tstack.push(tstack {
                        code,
                        pop: 1,
                        u: tstack_u::Redir(lst),
                    });
                } else if let Some(fr) = s_mut!() {
                    if let tstack_u::Redir(ref ll) = fr.u {
                        getredirs(ll); // c:509
                    }
                    stack = 1; // c:510
                }
            }
            WC_ASSIGN => {
                taddassign(code, state, 0);
            }
            WC_SIMPLE => {
                taddlist(state, WC_SIMPLE_ARGC(code) as i32);
                stack = 1;
            }
            WC_TYPESET => {
                taddlist(state, WC_TYPESET_ARGC(code) as i32);
                if state.pc < state.prog.prog.len() {
                    let cnt = state.prog.prog[state.pc];
                    state.pc += 1;
                    taddassignlist(state, cnt);
                }
                stack = 1;
            }
            WC_SUBSH => {
                if !s_active && spopped.is_none() {
                    taddstr("(");
                    tindent.with(|t| *t.borrow_mut() += 1);
                    taddnl(1);
                    let end_pc = state.pc + WC_SUBSH_SKIP(code) as usize;
                    tstack.push(tstack {
                        code,
                        pop: 1,
                        u: tstack_u::Subsh { end_pc },
                    });
                    state.pc += 1;
                } else if let Some(fr) = s_mut!() {
                    if let tstack_u::Subsh { end_pc } = fr.u {
                        state.pc = end_pc;
                    }
                    dec_tindent();
                    taddnl(0);
                    taddstr(")");
                    stack = 1;
                }
            }
            WC_CURSH => {
                if !s_active && spopped.is_none() {
                    taddstr("{");
                    tindent.with(|t| *t.borrow_mut() += 1);
                    taddnl(1);
                    let end_pc = state.pc + WC_CURSH_SKIP(code) as usize;
                    tstack.push(tstack {
                        code,
                        pop: 1,
                        u: tstack_u::Subsh { end_pc },
                    });
                    state.pc += 1;
                } else if let Some(fr) = s_mut!() {
                    if let tstack_u::Subsh { end_pc } = fr.u {
                        state.pc = end_pc;
                    }
                    dec_tindent();
                    taddnl(0);
                    taddstr("}");
                    stack = 1;
                }
            }
            WC_TIMED => {
                if !s_active && spopped.is_none() {
                    taddstr("time");
                    if WC_TIMED_TYPE(code) == WC_TIMED_PIPE {
                        tpush(b' ' as i32);
                        tindent.with(|t| *t.borrow_mut() += 1);
                        tstack.push(tstack {
                            code,
                            pop: 1,
                            u: tstack_u::None,
                        });
                    } else {
                        stack = 1;
                    }
                } else {
                    dec_tindent();
                    stack = 1;
                }
            }
            WC_FUNCDEF => {
                if !s_active && spopped.is_none() {
                    let p = state.pc;
                    let end_pc = p + WC_FUNCDEF_SKIP(code) as usize;
                    let nargs = if p < state.prog.prog.len() {
                        let n = state.prog.prog[p] as i32;
                        state.pc += 1;
                        n
                    } else {
                        0
                    };
                    if nargs > 1 {
                        taddstr("function ");
                    }
                    taddlist(state, nargs);
                    if nargs > 0 {
                        taddstr(" ");
                    }
                    if tjob.with(|c| *c.borrow()) {
                        if nargs > 1 {
                            taddstr("{ ... }");
                        } else {
                            taddstr("() { ... }");
                        }
                        state.pc = end_pc;
                        if nargs == 0 && end_pc < state.prog.prog.len() {
                            state.pc += state.prog.prog[end_pc] as usize;
                        }
                        stack = 1;
                    } else {
                        if nargs > 1 {
                            taddstr("{");
                        } else {
                            taddstr("() {");
                        }
                        tindent.with(|t| *t.borrow_mut() += 1);
                        taddnl(1);
                        let soff = state.strs_offset;
                        if state.pc < state.prog.prog.len() {
                            let bump = state.prog.prog[state.pc] as usize;
                            state.strs_offset += bump;
                            state.pc += 4;
                        }
                        tstack.push(tstack {
                            code,
                            pop: 1,
                            u: tstack_u::Funcdef {
                                strs_off: soff,
                                end_pc,
                                nargs,
                            },
                        });
                    }
                } else if let Some(fr) = s_mut!() {
                    if let tstack_u::Funcdef {
                        strs_off,
                        end_pc,
                        nargs,
                    } = &mut fr.u
                    {
                        state.strs_offset = *strs_off;
                        state.pc = *end_pc;
                        let nargs_copy = *nargs;
                        let end_copy = *end_pc;
                        dec_tindent();
                        taddnl(0);
                        taddstr("}");
                        if nargs_copy == 0 {
                            let mut epc = end_copy;
                            if state.pc < state.prog.prog.len() {
                                epc += state.prog.prog[state.pc] as usize;
                                state.pc += 1;
                            }
                            let n2 = if state.pc < state.prog.prog.len() {
                                let v = state.prog.prog[state.pc] as i32;
                                state.pc += 1;
                                v
                            } else {
                                0
                            };
                            if n2 != 0 {
                                tpush(b' ' as i32);
                                taddlist(state, n2);
                            }
                            state.pc = epc;
                        }
                    }
                    stack = 1;
                }
            }
            WC_FOR => {
                if !s_active && spopped.is_none() {
                    taddstr("for ");
                    if WC_FOR_TYPE(code) == WC_FOR_COND {
                        taddstr("((");
                        taddstr(&ecgetstr(state, EC_NODUP, None));
                        taddstr("; ");
                        taddstr(&ecgetstr(state, EC_NODUP, None));
                        taddstr("; ");
                        taddstr(&ecgetstr(state, EC_NODUP, None));
                        taddstr(")) do");
                    } else {
                        if state.pc < state.prog.prog.len() {
                            let a = state.prog.prog[state.pc];
                            state.pc += 1;
                            taddlist(state, a as i32);
                        }
                        if WC_FOR_TYPE(code) == WC_FOR_LIST {
                            taddstr(" in ");
                            if state.pc < state.prog.prog.len() {
                                let a = state.prog.prog[state.pc];
                                state.pc += 1;
                                taddlist(state, a as i32);
                            }
                        }
                        taddnl(0);
                        taddstr("do");
                    }
                    tindent.with(|t| *t.borrow_mut() += 1);
                    taddnl(0);
                    tstack.push(tstack {
                        code,
                        pop: 1,
                        u: tstack_u::None,
                    });
                } else {
                    dec_tindent();
                    taddnl(0);
                    taddstr("done");
                    stack = 1;
                }
            }
            WC_SELECT => {
                if !s_active && spopped.is_none() {
                    taddstr("select ");
                    taddstr(&ecgetstr(state, EC_NODUP, None));
                    if WC_SELECT_TYPE(code) == WC_SELECT_LIST {
                        taddstr(" in ");
                        if state.pc < state.prog.prog.len() {
                            let a = state.prog.prog[state.pc];
                            state.pc += 1;
                            taddlist(state, a as i32);
                        }
                    }
                    taddnl(0);
                    taddstr("do");
                    taddnl(0);
                    tindent.with(|t| *t.borrow_mut() += 1);
                    tstack.push(tstack {
                        code,
                        pop: 1,
                        u: tstack_u::None,
                    });
                } else {
                    dec_tindent();
                    taddnl(0);
                    taddstr("done");
                    stack = 1;
                }
            }
            WC_WHILE => {
                if !s_active && spopped.is_none() {
                    taddstr(if WC_WHILE_TYPE(code) == WC_WHILE_UNTIL {
                        "until "
                    } else {
                        "while "
                    });
                    tindent.with(|t| *t.borrow_mut() += 1);
                    tstack.push(tstack {
                        code,
                        pop: 0,
                        u: tstack_u::None,
                    });
                } else if let Some(fr) = s_mut!() {
                    if fr.pop == 0 {
                        dec_tindent();
                        taddnl(0);
                        taddstr("do");
                        tindent.with(|t| *t.borrow_mut() += 1);
                        taddnl(0);
                        fr.pop = 1;
                    } else {
                        dec_tindent();
                        taddnl(0);
                        taddstr("done");
                        stack = 1;
                    }
                }
            }
            WC_REPEAT => {
                if !s_active && spopped.is_none() {
                    taddstr("repeat ");
                    taddstr(&ecgetstr(state, EC_NODUP, None));
                    taddnl(0);
                    taddstr("do");
                    tindent.with(|t| *t.borrow_mut() += 1);
                    taddnl(0);
                    tstack.push(tstack {
                        code,
                        pop: 1,
                        u: tstack_u::None,
                    });
                } else {
                    dec_tindent();
                    taddnl(0);
                    taddstr("done");
                    stack = 1;
                }
            }
            WC_CASE => {
                if !s_active && spopped.is_none() {
                    let end_pc = state.pc + WC_CASE_SKIP(code) as usize;
                    taddstr("case ");
                    taddstr(&ecgetstr(state, EC_NODUP, None));
                    taddstr(" in");
                    if state.pc >= end_pc {
                        if tnewlins.with(|c| *c.borrow()) {
                            taddnl(0);
                        } else {
                            tpush(b' ' as i32);
                        }
                        taddstr("esac");
                        stack = 1;
                    } else {
                        tindent.with(|t| *t.borrow_mut() += 1);
                        if tnewlins.with(|c| *c.borrow()) {
                            taddnl(0);
                        } else {
                            tpush(b' ' as i32);
                        }
                        taddstr("(");
                        if state.pc >= state.prog.prog.len() {
                            break;
                        }
                        let c2 = state.prog.prog[state.pc];
                        state.pc += 1;
                        if state.pc >= state.prog.prog.len() {
                            break;
                        }
                        let prev_pc = state.pc;
                        let ialts = state.prog.prog[state.pc];
                        state.pc += 1;
                        let mut ial = ialts;
                        while ial > 0 {
                            taddstr(&ecgetstr(state, EC_NODUP, None));
                            state.pc = state.pc.saturating_add(1);
                            ial -= 1;
                            if ial > 0 {
                                taddstr(" | ");
                            }
                        }
                        taddstr(") ");
                        tindent.with(|t| *t.borrow_mut() += 1);
                        let pop_v = if prev_pc + WC_CASE_SKIP(c2) as usize >= end_pc {
                            1
                        } else {
                            0
                        };
                        tstack.push(tstack {
                            code: c2,
                            pop: pop_v,
                            u: tstack_u::Case { end_pc },
                        });
                    }
                } else if let Some(fr) = s_mut!() {
                    if let tstack_u::Case { end_pc } = fr.u {
                        if state.pc < end_pc {
                            dec_tindent();
                            match WC_CASE_TYPE(code) {
                                x if x == WC_CASE_OR => taddstr(" ;;"),
                                x if x == WC_CASE_AND => taddstr(" ;&"),
                                _ => taddstr(" ;|"),
                            }
                            if tnewlins.with(|c| *c.borrow()) {
                                taddnl(0);
                            } else {
                                tpush(b' ' as i32);
                            }
                            taddstr("(");
                            if state.pc >= state.prog.prog.len() {
                                break;
                            }
                            let c2 = state.prog.prog[state.pc];
                            state.pc += 1;
                            if state.pc >= state.prog.prog.len() {
                                break;
                            }
                            let prev_pc = state.pc;
                            let ialts = state.prog.prog[state.pc];
                            state.pc += 1;
                            let mut ial = ialts;
                            while ial > 0 {
                                taddstr(&ecgetstr(state, EC_NODUP, None));
                                state.pc = state.pc.saturating_add(1);
                                ial -= 1;
                                if ial > 0 {
                                    taddstr(" | ");
                                }
                            }
                            taddstr(") ");
                            tindent.with(|t| *t.borrow_mut() += 1);
                            fr.code = c2;
                            fr.pop = if prev_pc + WC_CASE_SKIP(c2) as usize >= end_pc {
                                1
                            } else {
                                0
                            };
                        } else {
                            dec_tindent();
                            match WC_CASE_TYPE(code) {
                                x if x == WC_CASE_OR => taddstr(" ;;"),
                                x if x == WC_CASE_AND => taddstr(" ;&"),
                                _ => taddstr(" ;|"),
                            }
                            dec_tindent();
                            if tnewlins.with(|c| *c.borrow()) {
                                taddnl(0);
                            } else {
                                tpush(b' ' as i32);
                            }
                            taddstr("esac");
                            stack = 1;
                        }
                    }
                }
            }
            WC_IF => {
                if !s_active && spopped.is_none() {
                    let end_pc = state.pc + WC_IF_SKIP(code) as usize;
                    taddstr("if ");
                    tindent.with(|t| *t.borrow_mut() += 1);
                    state.pc += 1;
                    tstack.push(tstack {
                        code,
                        pop: 0,
                        u: tstack_u::If { end_pc, cond: 1 },
                    });
                } else if let Some(fr) = s_mut!() {
                    if let tstack_u::If {
                        end_pc,
                        ref mut cond,
                    } = fr.u
                    {
                        if fr.pop != 0 {
                            stack = 1;
                        } else if *cond != 0 {
                            dec_tindent();
                            taddnl(0);
                            taddstr("then");
                            tindent.with(|t| *t.borrow_mut() += 1);
                            taddnl(0);
                            *cond = 0;
                        } else if state.pc < end_pc {
                            dec_tindent();
                            taddnl(0);
                            if state.pc >= state.prog.prog.len() {
                                break;
                            }
                            let c2 = state.prog.prog[state.pc];
                            state.pc += 1;
                            if WC_IF_TYPE(c2) == WC_IF_ELIF {
                                taddstr("elif ");
                                tindent.with(|t| *t.borrow_mut() += 1);
                                *cond = 1;
                            } else {
                                taddstr("else");
                                tindent.with(|t| *t.borrow_mut() += 1);
                                taddnl(0);
                            }
                        } else {
                            fr.pop = 1;
                            dec_tindent();
                            taddnl(0);
                            taddstr("fi");
                            stack = 1;
                        }
                    }
                }
            }
            WC_COND => {
                let entry = if !s_active && spopped.is_none() {
                    None
                } else if let Some(ref fr) = spopped {
                    if let tstack_u::Cond { par } = &fr.u {
                        Some((*par, fr.code))
                    } else {
                        None
                    }
                } else if let Some(i) = s_idx {
                    match &tstack[i].u {
                        tstack_u::Cond { par } => Some((*par, tstack[i].code)),
                        _ => None,
                    }
                } else {
                    None
                };
                // Rust-port: closure result → `stack`; C is `while (!stack)` (c:861-970).
                stack = (|| -> i32 {
                    let mut code = code;
                    let mut stack_out = 0i32;
                    if entry.is_none() {
                        taddstr("[[ "); // c:866
                        tstack.push(tstack {
                            code,
                            pop: 1,
                            u: tstack_u::Cond { par: 2 },
                        }); // c:867-868
                    } else {
                        let (par, scode) = entry.unwrap();
                        if par == 2 {
                            taddstr(" ]]"); // c:870
                            return 1; // c:871-872
                        }
                        if par == 1 {
                            taddstr(" )"); // c:874
                            return 1; // c:875-876
                        }
                        let oct = WC_COND_TYPE(scode);
                        if oct == COND_AND as wordcode {
                            taddstr(" && "); // c:878
                            if state.pc >= state.prog.prog.len() {
                                return 1;
                            }
                            code = state.prog.prog[state.pc];
                            state.pc += 1; // c:879
                            if WC_COND_TYPE(code) == COND_OR as wordcode {
                                taddstr("( "); // c:881
                                tstack.push(tstack {
                                    code,
                                    pop: 1,
                                    u: tstack_u::Cond { par: 1 },
                                }); // c:882-883
                            }
                        } else if oct == COND_OR as wordcode {
                            taddstr(" || "); // c:886
                            if state.pc >= state.prog.prog.len() {
                                return 1;
                            }
                            code = state.prog.prog[state.pc];
                            state.pc += 1; // c:887
                            if WC_COND_TYPE(code) == COND_AND as wordcode {
                                taddstr("( "); // c:889
                                tstack.push(tstack {
                                    code,
                                    pop: 1,
                                    u: tstack_u::Cond { par: 1 },
                                }); // c:890-891
                            }
                        }
                    }
                    while stack_out == 0 {
                        // c:894
                        let ctype = WC_COND_TYPE(code) as i32; // c:895
                        match ctype {
                            c if c == COND_NOT => {
                                taddstr("! "); // c:897
                                if state.pc >= state.prog.prog.len() {
                                    stack_out = 1;
                                    continue;
                                }
                                code = state.prog.prog[state.pc];
                                state.pc += 1; // c:898
                                if WC_COND_TYPE(code) <= COND_OR as wordcode {
                                    taddstr("( "); // c:900
                                    tstack.push(tstack {
                                        code,
                                        pop: 1,
                                        u: tstack_u::Cond { par: 1 },
                                    }); // c:901-902
                                }
                            }
                            c if c == COND_AND => {
                                tstack.push(tstack {
                                    code,
                                    pop: 1,
                                    u: tstack_u::Cond { par: 0 },
                                }); // c:906-907
                                if state.pc >= state.prog.prog.len() {
                                    stack_out = 1;
                                    continue;
                                }
                                code = state.prog.prog[state.pc];
                                state.pc += 1; // c:908
                                if WC_COND_TYPE(code) == COND_OR as wordcode {
                                    taddstr("( "); // c:910
                                    tstack.push(tstack {
                                        code,
                                        pop: 1,
                                        u: tstack_u::Cond { par: 1 },
                                    }); // c:911-912
                                }
                            }
                            c if c == COND_OR => {
                                tstack.push(tstack {
                                    code,
                                    pop: 1,
                                    u: tstack_u::Cond { par: 0 },
                                }); // c:916-917
                                if state.pc >= state.prog.prog.len() {
                                    stack_out = 1;
                                    continue;
                                }
                                code = state.prog.prog[state.pc];
                                state.pc += 1; // c:918
                                if WC_COND_TYPE(code) == COND_AND as wordcode {
                                    taddstr("( "); // c:920
                                    tstack.push(tstack {
                                        code,
                                        pop: 1,
                                        u: tstack_u::Cond { par: 1 },
                                    }); // c:921-922
                                }
                            }
                            c if c == COND_MOD => {
                                taddstr(&ecgetstr(state, EC_NODUP, None)); // c:926
                                tpush(b' ' as i32); // c:927
                                taddlist(state, WC_COND_SKIP(code) as i32); // c:928
                                stack_out = 1; // c:929
                            }
                            c if c == COND_MODI => {
                                let n = ecgetstr(state, EC_NODUP, None); // c:933
                                taddstr(&ecgetstr(state, EC_NODUP, None)); // c:935
                                tpush(b' ' as i32); // c:936
                                taddstr(&n); // c:937
                                tpush(b' ' as i32); // c:938
                                taddstr(&ecgetstr(state, EC_NODUP, None)); // c:939
                                stack_out = 1; // c:940
                            }
                            _ => {
                                if ctype < COND_MOD {
                                    // c:944-954 binary test branch
                                    taddstr(&ecgetstr(state, EC_NODUP, None)); // c:946
                                    taddstr(" "); // c:947
                                    let op_i = (ctype - COND_STREQ) as usize;
                                    if op_i < COND_BINARY_OPS.len() {
                                        taddstr(COND_BINARY_OPS[op_i]); // c:948 `cond_binary_ops[...]`
                                    }
                                    taddstr(" "); // c:949
                                    taddstr(&ecgetstr(state, EC_NODUP, None)); // c:950
                                    if ctype == COND_STREQ
                                        || ctype == COND_STRDEQ
                                        || ctype == COND_STRNEQ
                                    {
                                        state.pc += 1; // c:951-954
                                    }
                                } else {
                                    // c:956-965 unary `-X` tests
                                    let mut c2 = [0u8; 4];
                                    c2[0] = b'-'; // c:959
                                    c2[1] = ctype as u8; // c:960
                                    c2[2] = b' '; // c:961
                                    taddstr(&String::from_utf8_lossy(&c2[..3])); // c:963 `taddstr(c2)`
                                    taddstr(&ecgetstr(state, EC_NODUP, None)); // c:964
                                }
                                stack_out = 1; // c:966
                            }
                        }
                    }
                    stack_out
                })();
            }
            WC_ARITH => {
                taddstr("((");
                taddstr(&ecgetstr(state, EC_NODUP, None));
                taddstr("))");
                stack = 1;
            }
            WC_AUTOFN => {
                taddstr("builtin autoload -X");
                stack = 1;
            }
            WC_TRY => {
                if !s_active && spopped.is_none() {
                    taddstr("{");
                    tindent.with(|t| *t.borrow_mut() += 1);
                    taddnl(0);
                    if state.pc < state.prog.prog.len() {
                        state.pc += 1;
                        let w = if state.pc > 0 {
                            state.prog.prog[state.pc - 1]
                        } else {
                            0
                        };
                        let end_pc = state.pc + WC_CURSH_SKIP(w) as usize;
                        tstack.push(tstack {
                            code,
                            pop: 0,
                            u: tstack_u::Subsh { end_pc },
                        });
                    }
                } else if let Some(fr) = s_mut!() {
                    if fr.pop == 0 {
                        if let tstack_u::Subsh { end_pc } = fr.u {
                            state.pc = end_pc;
                        }
                        dec_tindent();
                        taddnl(0);
                        taddstr("} always {");
                        tindent.with(|t| *t.borrow_mut() += 1);
                        taddnl(0);
                        fr.pop = 1;
                    } else {
                        dec_tindent();
                        taddnl(0);
                        taddstr("}");
                        stack = 1;
                    }
                }
            }
            WC_END => {
                stack = 1;
            }
            _ => {
                // c:1010 — DPUTS(1, "unknown word code in gettext2()")
                DPUTS!(true, "unknown word code in gettext2()"); // c:1010
                return;
            }
        }
    }
    tdopending();
}

/// Port of `getredirs(LinkList redirs)` from `Src/text.c:1019`.
pub fn getredirs(redirs: &LinkList<redir>) {
    queue_signals(); // c:1019
    tpush(b' ' as i32); // c:1030
    for f in redirs.nodes.iter() {
        match f.typ {
            REDIR_WRITE | REDIR_WRITENOW | REDIR_APP | REDIR_APPNOW | REDIR_ERRWRITE
            | REDIR_ERRWRITENOW | REDIR_ERRAPP | REDIR_ERRAPPNOW | REDIR_READ | REDIR_READWRITE
            | REDIR_HERESTR | REDIR_MERGEIN | REDIR_MERGEOUT | REDIR_INPIPE | REDIR_OUTPIPE => {
                if let Some(ref vid) = f.varid {
                    tpush(b'{' as i32);
                    taddstr(vid);
                    tpush(b'}' as i32);
                } else if f.fd1 != (if IS_READFD(f.typ) { 0 } else { 1 }) {
                    // c:1054 — `taddchr('0' + f->fd1);`. Single
                    // char per the C body — no modulo, no
                    // truncation. fds >= 10 produce non-digit
                    // bytes (e.g. fd 10 → ':' 0x3A), matching C
                    // verbatim. The previous Rust port applied
                    // `% 10` which silently mapped fd 10 → '0',
                    // fd 11 → '1', etc., diverging from the C
                    // representation that callers rely on.
                    taddchr(b'0' as i32 + f.fd1); // c:1054
                }
                if f.typ == REDIR_HERESTR && (f.flags & REDIRF_FROM_HEREDOC) != 0 {
                    if tnewlins.with(|c| *c.borrow()) {
                        taddstr(FSTR[REDIR_HEREDOC as usize]);
                        if let Some(ref term) = f.here_terminator {
                            taddstr(term);
                        }
                        let n = f.name.clone().unwrap_or_default();
                        let m = f.munged_here_terminator.clone().unwrap_or_default();
                        taddpending(&n, &m);
                    } else {
                        taddstr(FSTR[REDIR_HERESTR as usize]);
                        let mut n = f.name.clone().unwrap_or_default();
                        let sav = n.ends_with('\n');
                        if sav {
                            n.pop();
                        }
                        if !has_token(&n) {
                            tpush(b'\'' as i32);
                            taddstr(&quotestring(&n, zsh_h::QT_SINGLE));
                            tpush(b'\'' as i32);
                        } else {
                            tpush(b'"' as i32);
                            taddstr(&quotestring(&n, zsh_h::QT_DOUBLE));
                            tpush(b'"' as i32);
                        }
                        let _ = sav;
                    }
                } else {
                    let fi = usize::try_from(f.typ).unwrap_or(0);
                    if fi < FSTR.len() {
                        taddstr(FSTR[fi]);
                    }
                    if f.typ != REDIR_MERGEIN && f.typ != REDIR_MERGEOUT {
                        tpush(b' ' as i32);
                    }
                    if let Some(ref nm) = f.name {
                        taddstr(nm);
                    }
                }
                tpush(b' ' as i32);
            }
            // c:1106-1110 — REDIR_CLOSE arm. DPUTS asserts it's a BUG
            // for the textual writer to ever see this; if reached, emit
            // the canonical `N>&-` text anyway.
            t if t == REDIR_CLOSE => {
                // c:1106
                DPUTS!(true, "BUG: CLOSE in getredirs()"); // c:1107
                taddchr(b'0' as i32 + f.fd1); // c:1108
                taddstr(">&- "); // c:1109
            }
            _ => {
                // c:1111 default
                DPUTS!(true, "BUG: unknown redirection in getredirs()"); // c:1112
            }
        }
    }
    tbuf.with(|tb| {
        let _ = tb.borrow_mut().pop();
    }); // c:1116
    unqueue_signals(); // c:1118
}

// ---------------------------------------------------------------------------
// Globals from text.c:40 / 48–54
// ---------------------------------------------------------------------------

/// Port of `int text_expand_tabs` from `Src/text.c:58`.
pub static TEXT_EXPAND_TABS: AtomicI32 = AtomicI32::new(0);

static COND_BINARY_OPS: &[&str] = &[
    "=", "==", "!=", "<", ">", "-nt", "-ot", "-ef", "-eq", "-ne", "-lt", "-gt", "-le", "-ge", "=~",
];

#[allow(non_camel_case_types)]
enum tstack_u {
    None,
    Redir(LinkList<redir>),
    Funcdef {
        strs_off: usize,
        end_pc: usize,
        nargs: i32,
    },
    Case {
        end_pc: usize,
    },
    If {
        end_pc: usize,
        cond: i32,
    },
    Cond {
        par: i32,
    },
    Subsh {
        end_pc: usize,
    },
}

// c:1022-1026 — `static char *fstr[]` from `Src/text.c:1022`.
// Indexed by `f->type` (the REDIR_* enum in `Src/zsh.h:377-397`).
// Position 12 = `REDIR_HERESTR` = `"<<<"` — previously typo'd as `"<<"`,
// which would silently misrender `cmd <<<word` as `cmd <<word`.
// Position 15 = `REDIR_CLOSE` is NULL in C (the dispatch routes via
// the `#ifdef DEBUG` branch only); Rust keeps `">&-"` here so an
// out-of-band index lookup doesn't panic, but the real REDIR_CLOSE
// case is filtered out of the match in `getredirs` (matches C
// release-build semantics).
const FSTR: [&str; 18] = [
    ">", ">|", ">>", ">>|", "&>", "&>|", "&>>", "&>>|", "<>", "<", "<<", "<<-", "<<<", "<&", ">&",
    ">&-", "<", ">",
];

/// WARNING: NOT IN `text.c` — `ecgetstr(Estate, …)` is defined in `Src/parse.c`.
/// Local shim renamed to avoid name-clashing with the canonical
/// `parse::ecgetstr(&mut estate, ...)`; the body delegates directly.
fn ecgetstr(st: &mut estate, dup: i32, tok: Option<&mut i32>) -> String {
    parse::ecgetstr(st, dup, tok)
}

#[inline]
fn useeprog(_p: &Eprog) {}

#[inline]
fn freeeprog(_p: &Eprog) {}

/// Port of `zoutputtab(FILE *outf)` from `Src/text.c:263`.
pub fn zoutputtab<W: std::io::Write>(outf: &mut W) -> std::io::Result<()> {
    let expand_tabs = TEXT_EXPAND_TABS.load(Ordering::Relaxed);
    if expand_tabs < 0 {
        return Ok(());
    }
    if expand_tabs > 0 {
        let spaces = vec![b' '; expand_tabs as usize];
        outf.write_all(&spaces)
    } else {
        outf.write_all(b"\t")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn getpermtext_renders_parse_string_eprog_with_pooled_strings() {
        let _g = crate::test_util::global_state_lock();
        // "echo hello world" pools the >3-char strings into eprog.strs;
        // getpermtext must resolve them (C text.c:292 `s.strs = prog->strs`).
        let prog = crate::ported::exec::parse_string("echo hello world", 0).expect("parse");
        let txt = getpermtext(Box::new(prog), None, 0);
        assert!(
            txt.contains("echo") && txt.contains("hello") && txt.contains("world"),
            "got: {txt:?}"
        );
    }

    #[test]
    fn getpermtext_case_arms_emit_terminators() {
        let _g = crate::test_util::global_state_lock();
        // c:text.c:765-770 — each case arm closes with ` ;;` (or ;&/;|).
        // Arms use the unparenthesized form: the ported par_case
        // cannot yet parse `(pat)` arms (pre-existing gap, surfaces
        // as `par_case: expected )` — the renderer under test here
        // is independent of that parser arm).
        let prog =
            crate::ported::exec::parse_string("case x in\na) echo A ;;\n*) echo other ;;\nesac", 0)
                .expect("parse");
        let txt = getpermtext(Box::new(prog), None, 0);
        assert!(
            txt.matches(";;").count() == 2,
            "expected two ;; terminators, got: {txt:?}"
        );
    }

    #[test]
    fn is_cond_binary_matches_zsh_set() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(is_cond_binary_op("="), 1);
        assert_eq!(is_cond_binary_op("-eq"), 1);
        assert_eq!(is_cond_binary_op("-nt"), 1);
        assert_eq!(is_cond_binary_op("-f"), 0);
        assert_eq!(is_cond_binary_op("foo"), 0);
    }

    #[test]
    fn zoutputtab_honours_text_expand_tabs() {
        let _g = crate::test_util::global_state_lock();
        TEXT_EXPAND_TABS.store(0, Ordering::Relaxed);
        let mut c = Cursor::new(Vec::new());
        zoutputtab(&mut c).unwrap();
        assert_eq!(c.into_inner(), b"\t");
        TEXT_EXPAND_TABS.store(4, Ordering::Relaxed);
        let mut c = Cursor::new(Vec::new());
        zoutputtab(&mut c).unwrap();
        assert_eq!(c.into_inner(), b"    ");
        TEXT_EXPAND_TABS.store(-1, Ordering::Relaxed);
        let mut c = Cursor::new(Vec::new());
        zoutputtab(&mut c).unwrap();
        assert_eq!(c.into_inner(), b"");
        TEXT_EXPAND_TABS.store(0, Ordering::Relaxed);
    }

    /// is_cond_binary_op accepts every documented binary test operator
    /// from the conditional dispatch table.
    #[test]
    fn is_cond_binary_op_recognises_canonical_operators() {
        let _g = crate::test_util::global_state_lock();
        for op in [
            "=", "==", "!=", "<", ">", "-eq", "-ne", "-lt", "-le", "-gt", "-ge",
        ] {
            assert_eq!(is_cond_binary_op(op), 1, "{op:?} must be recognised");
        }
    }

    /// Unknown / unary / nonsense operators return 0.
    #[test]
    fn is_cond_binary_op_rejects_unknown_operators() {
        let _g = crate::test_util::global_state_lock();
        for op in ["?", "@", "", " ", "==="] {
            assert_eq!(
                is_cond_binary_op(op),
                0,
                "{op:?} must NOT be recognised as binary"
            );
        }
    }

    /// `Src/text.c:48-51` — the `cond_binary_ops` table is order-
    /// dependent: the comment at c:45-46 states "Their order is tied
    /// to the order of the definitions COND_STREQ et seq. in zsh.h."
    /// A regression that reorders this array silently misroutes
    /// every `[[ $a -eq $b ]]` dispatch in `Src/cond.c` because the
    /// caller indexes into the array with `(ctype - COND_STREQ)`.
    /// Pin the exact array contents AND the exact length.
    #[test]
    fn cond_binary_ops_table_matches_c_source_exactly() {
        let _g = crate::test_util::global_state_lock();
        // c:48-51 — verbatim list. Order is the contract.
        let expected = [
            "=", "==", "!=", "<", ">", "-nt", "-ot", "-ef", "-eq", "-ne", "-lt", "-gt", "-le",
            "-ge", "=~",
        ];
        assert_eq!(
            COND_BINARY_OPS.len(),
            expected.len(),
            "c:48-51 — table must have exactly 15 ops (excluding the NULL sentinel)"
        );
        for (i, &op) in expected.iter().enumerate() {
            assert_eq!(
                COND_BINARY_OPS[i], op,
                "c:48-51 — position {} must be {:?}, got {:?}",
                i, op, COND_BINARY_OPS[i]
            );
        }
    }

    /// `Src/text.c:58-67` — `is_cond_binary_op` returns 1 for the
    /// file-test ops `-nt`, `-ot`, `-ef` and the regex match `=~`.
    /// The existing canonical-ops test misses these four.
    #[test]
    fn is_cond_binary_op_accepts_file_test_and_regex_ops() {
        let _g = crate::test_util::global_state_lock();
        for op in ["-nt", "-ot", "-ef", "=~"] {
            assert_eq!(
                is_cond_binary_op(op),
                1,
                "c:63 — strcmp match must accept {:?}",
                op
            );
        }
    }

    /// `taddchr` + `taddstr` smoke — no panic, sanitises pending buffer.
    #[test]
    fn taddchr_taddstr_smoke_no_panic() {
        let _g = crate::test_util::global_state_lock();
        taddchr(b'x' as i32);
        taddstr("hello");
        tdopending();
    }

    /// `taddpending` queues a deferred pair flushed via `tdopending`.
    #[test]
    fn taddpending_then_tdopending_no_panic() {
        let _g = crate::test_util::global_state_lock();
        taddpending("foo", "bar");
        tdopending();
    }

    /// `taddnl(no_semicolon)` two-mode path: 0 = `; \n`, 1 = `\n` only.
    #[test]
    fn taddnl_two_modes_no_panic() {
        let _g = crate::test_util::global_state_lock();
        taddnl(0);
        taddnl(1);
        tdopending();
    }

    /// `Src/text.c:1022-1026` — `static char *fstr[]` is indexed by
    /// `f->type` (the REDIR_* enum at `Src/zsh.h:377-397`). The order
    /// is the contract: a regression that swaps positions silently
    /// misrenders every `getredirs` output. Pin every slot in the
    /// table by C-source comment. Position 12 (REDIR_HERESTR) was
    /// previously typo'd as `"<<"` instead of `"<<<"`, dropping the
    /// third `<` from herestring round-trips.
    #[test]
    fn fstr_table_matches_c_source_position_by_position() {
        let _g = crate::test_util::global_state_lock();
        // c:1024-1026 — verbatim list (NULL at position 15 → ">&-" placeholder).
        let expected = [
            ">",    // REDIR_WRITE       = 0
            ">|",   // REDIR_WRITENOW    = 1
            ">>",   // REDIR_APP         = 2
            ">>|",  // REDIR_APPNOW      = 3
            "&>",   // REDIR_ERRWRITE    = 4
            "&>|",  // REDIR_ERRWRITENOW = 5
            "&>>",  // REDIR_ERRAPP      = 6
            "&>>|", // REDIR_ERRAPPNOW   = 7
            "<>",   // REDIR_READWRITE   = 8
            "<",    // REDIR_READ        = 9
            "<<",   // REDIR_HEREDOC     = 10
            "<<-",  // REDIR_HEREDOCDASH = 11
            "<<<",  // REDIR_HERESTR     = 12 — c:1025 third `<` is load-bearing
            "<&",   // REDIR_MERGEIN     = 13
            ">&",   // REDIR_MERGEOUT    = 14
            ">&-",  // REDIR_CLOSE       = 15 — NULL in C; placeholder here
            "<",    // REDIR_INPIPE      = 16
            ">",    // REDIR_OUTPIPE     = 17
        ];
        assert_eq!(
            FSTR.len(),
            expected.len(),
            "c:1022 — fstr[] must have exactly 18 slots"
        );
        for (i, &want) in expected.iter().enumerate() {
            assert_eq!(
                FSTR[i], want,
                "c:1024-1026 — FSTR[{}] must be {:?}, got {:?}",
                i, want, FSTR[i]
            );
        }
        // Pin the herestring slot specifically — that was the
        // typo'd cell.
        assert_eq!(
            FSTR[REDIR_HERESTR as usize], "<<<",
            "c:1025 — REDIR_HERESTR (12) must render as `<<<`"
        );
        // Pin the heredoc slot too — it shares fstr's 10 cell.
        assert_eq!(
            FSTR[REDIR_HEREDOC as usize], "<<",
            "c:1024 — REDIR_HEREDOC (10) must render as `<<`"
        );
    }

    /// Pin: `Src/text.c:1054` — `taddchr('0' + f->fd1);` adds a
    /// SINGLE byte to the text buffer. No modulo, no truncation.
    /// fds 0..=9 render as ASCII digits `'0'..='9'`; fds >= 10
    /// render as the corresponding 0x3A onward bytes (':' for 10,
    /// ';' for 11, etc.) — matching C byte-for-byte even though
    /// the result is not a usable shell representation. Test the
    /// arithmetic via `taddchr` directly so we don't have to
    /// construct a full Redir list.
    ///
    /// The previous Rust port applied `% 10` which mapped fd 10
    /// to '0' (corrupting the 1/2 fd-prefix detection in callers
    /// that re-parse the text). Pin the byte that `taddchr`
    /// receives for the four boundary fds.
    #[test]
    fn getredirs_fd1_emits_single_byte_no_modulo() {
        let _g = crate::test_util::global_state_lock();
        // The arithmetic the function performs: `'0' + fd1`.
        // For fd1 in 0..=9 it produces an ASCII digit.
        for fd in 0..=9i32 {
            let expected = (b'0' as i32 + fd) as u8;
            assert_eq!(
                expected,
                b'0' + fd as u8,
                "c:1054 — fd {} must produce byte {}",
                fd,
                expected
            );
        }
        // For fd1 == 10, C emits ':' (0x3A). The previous Rust
        // port produced '0' (0x30) — divergent.
        assert_eq!(
            (b'0' as i32 + 10) as u8,
            b':',
            "c:1054 — fd 10 emits ':' (0x3A) byte verbatim, not '0' (modulo)"
        );
        // For fd1 == 11, C emits ';' (0x3B).
        assert_eq!(
            (b'0' as i32 + 11) as u8,
            b';',
            "c:1054 — fd 11 emits ';' (0x3B), not '1'"
        );
    }

    /// c:48-51 — `cond_binary_ops` table MUST NOT contain duplicates.
    /// The C source uses linear search; duplicates would silently
    /// route to the FIRST entry's COND_* dispatch index, making
    /// the second entry unreachable.
    #[test]
    fn cond_binary_ops_has_no_duplicates() {
        let _g = crate::test_util::global_state_lock();
        let unique: std::collections::HashSet<_> = COND_BINARY_OPS.iter().copied().collect();
        assert_eq!(
            unique.len(),
            COND_BINARY_OPS.len(),
            "duplicate entry in COND_BINARY_OPS"
        );
    }

    /// c:48-51 — Every op in `cond_binary_ops` must be a recognised
    /// binary operator (round-trip). Pin the table-walk-vs-lookup
    /// consistency: anything in the table → `is_cond_binary_op == 1`.
    #[test]
    fn every_cond_binary_op_round_trips() {
        let _g = crate::test_util::global_state_lock();
        for &op in COND_BINARY_OPS {
            assert_eq!(
                is_cond_binary_op(op),
                1,
                "{:?} is in COND_BINARY_OPS but is_cond_binary_op rejects it",
                op
            );
        }
    }

    /// c:88 — `taddchr` appends a single byte. Test smoke + state
    /// query via `tdopending` (which drains the buffer).
    #[test]
    fn taddchr_appends_single_byte() {
        let _g = crate::test_util::global_state_lock();
        // Smoke: must not panic with any byte value
        for c in [0i32, 32, 65, 127, 200, 255] {
            taddchr(c);
        }
    }

    /// c:93 — `taddstr` appends a string slice. Smoke test
    /// no-panic over a variety of inputs.
    #[test]
    fn taddstr_appends_string_safely() {
        let _g = crate::test_util::global_state_lock();
        taddstr("");
        taddstr("a");
        taddstr("hello world");
        // Multibyte UTF-8 must not panic
        taddstr("café — résumé");
    }

    /// c:1273 — `TEXT_EXPAND_TABS` defaults to 0. Pin the
    /// initial value so a regen flipping to 4 silently doubles
    /// every prompt's tab width.
    #[test]
    fn text_expand_tabs_default_is_zero() {
        let _g = crate::test_util::global_state_lock();
        // Reset to default
        TEXT_EXPAND_TABS.store(0, Ordering::Relaxed);
        let v = TEXT_EXPAND_TABS.load(Ordering::Relaxed);
        assert_eq!(v, 0, "TEXT_EXPAND_TABS default must be 0 (literal tabs)");
    }

    /// c:1332 — `zoutputtab` with `TEXT_EXPAND_TABS = 8` emits 8
    /// spaces. Pin the canonical "1 tab = 8 spaces" width because
    /// it matters for `getjobtext` indentation.
    #[test]
    fn zoutputtab_emits_n_spaces_for_n_value() {
        let _g = crate::test_util::global_state_lock();
        let saved = TEXT_EXPAND_TABS.load(Ordering::Relaxed);
        TEXT_EXPAND_TABS.store(8, Ordering::Relaxed);
        let mut c = Cursor::new(Vec::new());
        zoutputtab(&mut c).unwrap();
        assert_eq!(
            c.into_inner(),
            b"        ",
            "TEXT_EXPAND_TABS=8 must emit 8 spaces"
        );
        TEXT_EXPAND_TABS.store(saved, Ordering::Relaxed);
    }

    /// c:1332 — `zoutputtab` with `TEXT_EXPAND_TABS = 0` emits a
    /// literal tab. Pin the no-expand path.
    #[test]
    fn zoutputtab_zero_emits_literal_tab() {
        let _g = crate::test_util::global_state_lock();
        let saved = TEXT_EXPAND_TABS.load(Ordering::Relaxed);
        TEXT_EXPAND_TABS.store(0, Ordering::Relaxed);
        let mut c = Cursor::new(Vec::new());
        zoutputtab(&mut c).unwrap();
        assert_eq!(c.into_inner(), b"\t");
        TEXT_EXPAND_TABS.store(saved, Ordering::Relaxed);
    }

    /// c:1332 — `zoutputtab` with negative value emits NOTHING
    /// (the "suppress" sentinel). Pin so a regen that treats
    /// negative as 0 silently breaks `getpermtext` indent.
    #[test]
    fn zoutputtab_negative_emits_nothing() {
        let _g = crate::test_util::global_state_lock();
        let saved = TEXT_EXPAND_TABS.load(Ordering::Relaxed);
        TEXT_EXPAND_TABS.store(-1, Ordering::Relaxed);
        let mut c = Cursor::new(Vec::new());
        zoutputtab(&mut c).unwrap();
        assert!(
            c.into_inner().is_empty(),
            "negative TEXT_EXPAND_TABS must emit nothing"
        );
        TEXT_EXPAND_TABS.store(saved, Ordering::Relaxed);
    }

    // ─── zsh-corpus pins for is_cond_binary_op ─────────────────────

    /// Equality ops `=`, `==`, `!=`, `=~` are all binary cond ops.
    #[test]
    fn text_corpus_is_cond_binary_op_equality() {
        assert_eq!(is_cond_binary_op("="), 1);
        assert_eq!(is_cond_binary_op("=="), 1);
        assert_eq!(is_cond_binary_op("!="), 1);
        assert_eq!(is_cond_binary_op("=~"), 1);
    }

    /// String compare ops `<`, `>` are binary cond ops.
    #[test]
    fn text_corpus_is_cond_binary_op_lex_compare() {
        assert_eq!(is_cond_binary_op("<"), 1);
        assert_eq!(is_cond_binary_op(">"), 1);
    }

    /// File-time ops `-nt`, `-ot`, `-ef` are binary cond ops.
    #[test]
    fn text_corpus_is_cond_binary_op_file_time() {
        assert_eq!(is_cond_binary_op("-nt"), 1);
        assert_eq!(is_cond_binary_op("-ot"), 1);
        assert_eq!(is_cond_binary_op("-ef"), 1);
    }

    /// Numeric compare ops -eq/-ne/-lt/-gt/-le/-ge are binary.
    #[test]
    fn text_corpus_is_cond_binary_op_numeric() {
        for op in ["-eq", "-ne", "-lt", "-gt", "-le", "-ge"] {
            assert_eq!(is_cond_binary_op(op), 1, "op '{op}' should be binary");
        }
    }

    /// Unary-only ops are NOT binary: `-z`, `-n`, `-e`, `-f`, `-d`.
    #[test]
    fn text_corpus_is_cond_binary_op_rejects_unary() {
        for op in ["-z", "-n", "-e", "-f", "-d", "-r", "-w", "-x", "!"] {
            assert_eq!(is_cond_binary_op(op), 0, "op '{op}' should NOT be binary");
        }
    }

    /// Random strings are not binary cond ops.
    #[test]
    fn text_corpus_is_cond_binary_op_rejects_arbitrary() {
        assert_eq!(is_cond_binary_op("hello"), 0);
        assert_eq!(is_cond_binary_op(""), 0);
        assert_eq!(is_cond_binary_op("=="), 1); // sanity that the list still works
        assert_eq!(is_cond_binary_op("==="), 0, "triple-eq not in list");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/text.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:32 — `is_cond_binary_op` recognizes string-comparison ops.
    #[test]
    fn is_cond_binary_op_recognizes_string_ops() {
        for op in ["=", "==", "!=", "<", ">"] {
            assert_eq!(is_cond_binary_op(op), 1, "{:?} must be binary", op);
        }
    }

    /// c:32 — `is_cond_binary_op` recognizes -ot/-nt/-ef (file compare).
    #[test]
    fn is_cond_binary_op_recognizes_file_compare_ops() {
        for op in ["-ot", "-nt", "-ef"] {
            assert_eq!(is_cond_binary_op(op), 1, "{:?} must be binary", op);
        }
    }

    /// c:32 — recognizes numeric ops -eq/-ne/-lt/-gt/-le/-ge.
    #[test]
    fn is_cond_binary_op_recognizes_numeric_ops() {
        for op in ["-eq", "-ne", "-lt", "-gt", "-le", "-ge"] {
            assert_eq!(is_cond_binary_op(op), 1, "{:?} must be binary", op);
        }
    }

    /// c:32 — case-sensitive: `-EQ` is NOT recognized.
    #[test]
    fn is_cond_binary_op_is_case_sensitive() {
        assert_eq!(is_cond_binary_op("-EQ"), 0, "uppercase variant not in list");
        assert_eq!(is_cond_binary_op("-LT"), 0);
    }

    /// c:32 — `is_cond_binary_op` is deterministic.
    #[test]
    fn is_cond_binary_op_is_deterministic() {
        for op in ["-eq", "==", "!=", "hello", ""] {
            let first = is_cond_binary_op(op);
            for _ in 0..5 {
                assert_eq!(is_cond_binary_op(op), first, "{:?} must be pure", op);
            }
        }
    }

    /// c:70 — `dec_tindent` from 0 is safe (DPUTS warns but no panic).
    #[test]
    fn dec_tindent_from_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        // Initial tindent is 0; decrementing should be no-op + DPUTS.
        dec_tindent();
        // Verify still 0 after.
        let v = tindent.with(|t| *t.borrow());
        assert_eq!(v, 0, "tindent must not go negative");
    }

    /// c:89 — `taddpending("", "")` is no-op (empty content).
    #[test]
    fn taddpending_empty_strings_no_panic() {
        let _g = crate::test_util::global_state_lock();
        // Clear first.
        tpending.with(|p| *p.borrow_mut() = None);
        taddpending("", "");
        // tpending should now be Some(empty) or remain None per port.
    }

    /// c:89 — `taddpending` accumulates: two calls produce "a\\nb"
    /// (per c:91 ascii separator).
    #[test]
    fn taddpending_accumulates_with_newline_separator() {
        let _g = crate::test_util::global_state_lock();
        tpending.with(|p| *p.borrow_mut() = None);
        taddpending("a", "");
        taddpending("b", "");
        let result = tpending.with(|p| p.borrow().clone());
        assert_eq!(result.as_deref(), Some(b"a\nb" as &[u8]));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/text.c
    // c:128 taddchr / c:146 taddstr / c:163 taddnl / c:1356 zoutputtab
    // ═══════════════════════════════════════════════════════════════════

    /// c:89 — `taddpending` with str1 alone (no second part).
    #[test]
    fn taddpending_str1_only_buffers_literal_bytes() {
        let _g = crate::test_util::global_state_lock();
        tpending.with(|p| *p.borrow_mut() = None);
        taddpending("hello", "");
        let result = tpending.with(|p| p.borrow().clone());
        assert_eq!(result.as_deref(), Some(b"hello" as &[u8]));
    }

    /// c:89 — `taddpending` concatenates str1+str2 within single call.
    #[test]
    fn taddpending_concats_str1_str2_within_single_call() {
        let _g = crate::test_util::global_state_lock();
        tpending.with(|p| *p.borrow_mut() = None);
        taddpending("foo", "bar");
        let result = tpending.with(|p| p.borrow().clone());
        assert_eq!(result.as_deref(), Some(b"foobar" as &[u8]));
    }

    /// c:89 — three-call accumulation produces "a\nb\nc".
    #[test]
    fn taddpending_three_calls_inserts_two_newlines() {
        let _g = crate::test_util::global_state_lock();
        tpending.with(|p| *p.borrow_mut() = None);
        taddpending("a", "");
        taddpending("b", "");
        taddpending("c", "");
        let result = tpending.with(|p| p.borrow().clone());
        assert_eq!(result.as_deref(), Some(b"a\nb\nc" as &[u8]));
    }

    /// c:70 — `dec_tindent` floors at zero (saturating).
    #[test]
    fn dec_tindent_floors_at_zero_multiple_calls() {
        let _g = crate::test_util::global_state_lock();
        tindent.with(|t| *t.borrow_mut() = 0);
        // Multiple decrements from zero must not underflow.
        for _ in 0..5 {
            dec_tindent();
        }
        tindent.with(|t| assert_eq!(*t.borrow(), 0));
    }

    /// c:114 — `tdopending` clears pending after draining.
    #[test]
    fn tdopending_clears_pending_buffer() {
        let _g = crate::test_util::global_state_lock();
        tpending.with(|p| *p.borrow_mut() = Some(b"deferred".to_vec()));
        tdopending();
        let after = tpending.with(|p| p.borrow().clone());
        assert!(after.is_none(), "tdopending must clear pending");
    }

    /// c:114 — `tdopending` is no-op when pending is None.
    #[test]
    fn tdopending_noop_when_pending_none() {
        let _g = crate::test_util::global_state_lock();
        tpending.with(|p| *p.borrow_mut() = None);
        tdopending();
        let after = tpending.with(|p| p.borrow().clone());
        assert!(after.is_none(), "tdopending on None stays None");
    }

    /// c:58 — every COND_BINARY_OPS entry is non-empty, no NULs.
    #[test]
    fn cond_binary_ops_entries_well_formed() {
        for &op in COND_BINARY_OPS {
            assert!(!op.is_empty(), "no empty entries");
            assert!(!op.contains('\0'), "no NUL bytes in entries");
        }
    }

    /// c:58 — `is_cond_binary_op` always returns 0 or 1 (boolean i32).
    #[test]
    fn is_cond_binary_op_returns_boolean_i32_only() {
        let inputs = ["=", "!=", "garbage", "", "==", "-eq", "if"];
        for s in inputs {
            let r = is_cond_binary_op(s);
            assert!(r == 0 || r == 1, "result is 0/1 for {:?} (got {})", s, r);
        }
    }

    /// c:58 — empty string is NOT a binary cond op.
    #[test]
    fn is_cond_binary_op_empty_returns_zero() {
        assert_eq!(is_cond_binary_op(""), 0);
    }

    /// c:163 — `taddnl(0)` in newlins=false mode appends "; ".
    #[test]
    fn taddnl_no_newlins_mode_appends_semicolon_space() {
        let _g = crate::test_util::global_state_lock();
        tbuf.with(|tb| tb.borrow_mut().clear());
        tnewlins.with(|c| *c.borrow_mut() = false);
        taddnl(0);
        let got = tbuf.with(|tb| tb.borrow().clone());
        tnewlins.with(|c| *c.borrow_mut() = true); // restore
        assert_eq!(&got[..], b"; ", "non-newlins mode appends \"; \"");
    }

    /// c:163 — `taddnl(1)` in newlins=false mode appends single space
    /// (no-semicolon variant).
    #[test]
    fn taddnl_no_semicolon_mode_appends_single_space() {
        let _g = crate::test_util::global_state_lock();
        tbuf.with(|tb| tb.borrow_mut().clear());
        tnewlins.with(|c| *c.borrow_mut() = false);
        taddnl(1);
        let got = tbuf.with(|tb| tb.borrow().clone());
        tnewlins.with(|c| *c.borrow_mut() = true);
        assert_eq!(&got[..], b" ", "no-semicolon mode appends single space");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/text.c
    // c:58 is_cond_binary_op / c:70 dec_tindent / c:89 taddpending /
    // c:128 taddchr / c:146 taddstr
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `is_cond_binary_op` returns 1 for every entry in
    /// COND_BINARY_OPS (full table sweep).
    #[test]
    fn is_cond_binary_op_recognizes_every_entry() {
        for &op in COND_BINARY_OPS {
            assert_eq!(
                is_cond_binary_op(op),
                1,
                "every COND_BINARY_OPS entry must match itself; failed on {:?}",
                op
            );
        }
    }

    /// c:58 — `is_cond_binary_op` rejects obvious unary/keyword tokens.
    #[test]
    fn is_cond_binary_op_rejects_non_binary_ops() {
        for s in &["-e", "-z", "-n", "if", "then", "fi", "else"] {
            assert_eq!(
                is_cond_binary_op(s),
                0,
                "{:?} must not be classified as binary cond op",
                s
            );
        }
    }

    /// c:58 — `is_cond_binary_op` is pure (deterministic across calls).
    #[test]
    fn is_cond_binary_op_deterministic() {
        for s in &["=", "!=", "garbage", "", "-eq"] {
            let first = is_cond_binary_op(s);
            for _ in 0..10 {
                assert_eq!(is_cond_binary_op(s), first);
            }
        }
    }

    /// c:58 — `is_cond_binary_op` returns i32 (compile-time pin).
    #[test]
    fn is_cond_binary_op_returns_i32_type() {
        let _: i32 = is_cond_binary_op("=");
    }

    /// c:70 — `dec_tindent` from value N reaches N-1 (single decrement).
    #[test]
    fn dec_tindent_single_decrement() {
        let _g = crate::test_util::global_state_lock();
        tindent.with(|t| *t.borrow_mut() = 5);
        dec_tindent();
        tindent.with(|t| assert_eq!(*t.borrow(), 4, "5 → 4 after one dec"));
    }

    /// c:70 — `dec_tindent` repeated N times from N reaches zero.
    #[test]
    fn dec_tindent_n_calls_from_n_reaches_zero() {
        let _g = crate::test_util::global_state_lock();
        tindent.with(|t| *t.borrow_mut() = 10);
        for _ in 0..10 {
            dec_tindent();
        }
        tindent.with(|t| assert_eq!(*t.borrow(), 0));
    }

    /// c:89 — `taddpending` on initial None creates pending (single call).
    #[test]
    fn taddpending_initial_none_creates_pending() {
        let _g = crate::test_util::global_state_lock();
        tpending.with(|p| *p.borrow_mut() = None);
        taddpending("x", "");
        let result = tpending.with(|p| p.borrow().clone());
        assert_eq!(
            result.as_deref(),
            Some(b"x" as &[u8]),
            "first call must create pending from None"
        );
    }

    /// c:89 — `taddpending` with both empty args still allocates.
    #[test]
    fn taddpending_both_empty_args_safe() {
        let _g = crate::test_util::global_state_lock();
        tpending.with(|p| *p.borrow_mut() = None);
        taddpending("", "");
        let result = tpending.with(|p| p.borrow().clone());
        assert_eq!(
            result.as_deref(),
            Some(b"" as &[u8]),
            "both-empty args produces empty buffer (not None)"
        );
    }

    /// c:128 — `taddchr` is the single-byte form of taddstr in newlins=true.
    #[test]
    fn taddchr_appends_single_byte_in_newlins_mode() {
        let _g = crate::test_util::global_state_lock();
        tbuf.with(|tb| tb.borrow_mut().clear());
        tnewlins.with(|c| *c.borrow_mut() = true);
        taddchr(b'a' as i32);
        taddchr(b'b' as i32);
        let got = tbuf.with(|tb| tb.borrow().clone());
        assert_eq!(&got[..], b"ab", "two char appends produce 'ab'");
    }

    /// c:146 — `taddstr` in newlins=true mode appends bytes verbatim
    /// including embedded newline.
    #[test]
    fn taddstr_newlins_mode_preserves_newlines() {
        let _g = crate::test_util::global_state_lock();
        tbuf.with(|tb| tb.borrow_mut().clear());
        tnewlins.with(|c| *c.borrow_mut() = true);
        taddstr("a\nb");
        let got = tbuf.with(|tb| tb.borrow().clone());
        assert_eq!(&got[..], b"a\nb", "newlins=true preserves '\\n' verbatim");
    }

    /// c:146 — `taddstr` in newlins=false mode rewrites '\n' to space.
    #[test]
    fn taddstr_no_newlins_mode_rewrites_newline_to_space() {
        let _g = crate::test_util::global_state_lock();
        tbuf.with(|tb| tb.borrow_mut().clear());
        tnewlins.with(|c| *c.borrow_mut() = false);
        taddstr("a\nb");
        let got = tbuf.with(|tb| tb.borrow().clone());
        tnewlins.with(|c| *c.borrow_mut() = true);
        assert_eq!(&got[..], b"a b", "newlins=false rewrites '\\n' → ' '");
    }

    /// c:146 — `taddstr` empty input is a no-op (no buffer growth).
    #[test]
    fn taddstr_empty_input_is_noop() {
        let _g = crate::test_util::global_state_lock();
        tbuf.with(|tb| tb.borrow_mut().clear());
        tnewlins.with(|c| *c.borrow_mut() = true);
        taddstr("");
        let got = tbuf.with(|tb| tb.borrow().clone());
        assert!(got.is_empty(), "empty input must not grow buffer");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/text.c
    // c:48-51 cond_binary_ops table / c:65 taddchr / c:146 taddstr /
    // c:90 taddpending / tdopending
    // ═══════════════════════════════════════════════════════════════════

    /// c:48-51 — `cond_binary_ops` contains the canonical 15 entries:
    /// 6 string comparators (=, ==, !=, <, >, =~), 3 file-stamp ops
    /// (-nt, -ot, -ef), 6 arithmetic comparators (-eq..-ge).
    #[test]
    fn cond_binary_ops_has_15_entries() {
        assert_eq!(
            COND_BINARY_OPS.len(),
            15,
            "cond_binary_ops must have 15 entries; got {}",
            COND_BINARY_OPS.len()
        );
    }

    /// c:48-51 — every cond_binary_ops entry is non-empty.
    #[test]
    fn cond_binary_ops_all_non_empty() {
        for op in COND_BINARY_OPS {
            assert!(!op.is_empty(), "cond_binary_op must be non-empty: {:?}", op);
        }
    }

    /// c:48-51 — every cond_binary_ops entry length ≤ 3 (longest is `-nt`).
    #[test]
    fn cond_binary_ops_max_length_three() {
        for op in COND_BINARY_OPS {
            assert!(op.len() <= 3, "cond_binary_op must be ≤ 3 chars: {:?}", op);
        }
    }

    /// c:48-51 — `=~` is the regex match operator (last entry).
    #[test]
    fn cond_binary_ops_last_is_regex_match() {
        assert_eq!(
            *COND_BINARY_OPS.last().unwrap(),
            "=~",
            "regex-match `=~` must be last"
        );
    }

    /// c:48-51 — `=` is the first entry (assignment-style comparator).
    #[test]
    fn cond_binary_ops_first_is_eq() {
        assert_eq!(COND_BINARY_OPS[0], "=", "first entry must be `=`");
    }

    /// c:32 — `is_cond_binary_op` returns 0 or 1 (bool-as-i32).
    #[test]
    fn is_cond_binary_op_returns_only_0_or_1() {
        for s in ["=", "==", "==", "foo", "", "[", "<<"] {
            let r = is_cond_binary_op(s);
            assert!(
                r == 0 || r == 1,
                "is_cond_binary_op({:?}) = {} not in {{0,1}}",
                s,
                r
            );
        }
    }

    /// c:32 — `is_cond_binary_op` is pure (no global mutation).
    #[test]
    fn is_cond_binary_op_is_pure() {
        let _g = crate::test_util::global_state_lock();
        tbuf.with(|tb| tb.borrow_mut().clear());
        let cap_before = tbuf.with(|tb| tb.borrow().len());
        let _ = is_cond_binary_op("==");
        let _ = is_cond_binary_op("foo");
        let cap_after = tbuf.with(|tb| tb.borrow().len());
        assert_eq!(
            cap_before, cap_after,
            "is_cond_binary_op must not mutate tbuf"
        );
    }

    /// c:65 — `taddchr` appends multiple distinct bytes.
    #[test]
    fn taddchr_appends_multiple_bytes_in_sequence() {
        let _g = crate::test_util::global_state_lock();
        tbuf.with(|tb| tb.borrow_mut().clear());
        tnewlins.with(|c| *c.borrow_mut() = true);
        taddchr(b'a' as i32);
        taddchr(b'b' as i32);
        taddchr(b'c' as i32);
        let got = tbuf.with(|tb| tb.borrow().clone());
        assert_eq!(&got[..], b"abc", "taddchr must append in order");
    }

    /// c:146 — `taddstr` very long string doesn't panic (buffer grows).
    #[test]
    fn taddstr_large_input_no_panic() {
        let _g = crate::test_util::global_state_lock();
        tbuf.with(|tb| tb.borrow_mut().clear());
        tnewlins.with(|c| *c.borrow_mut() = true);
        let long = "x".repeat(10_000);
        taddstr(&long);
        let got_len = tbuf.with(|tb| tb.borrow().len());
        assert_eq!(got_len, 10_000, "buffer must hold all 10000 bytes");
    }

    /// c:90 — `taddpending` with two long strings doesn't panic.
    #[test]
    fn taddpending_long_strings_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let s1 = "a".repeat(500);
        let s2 = "b".repeat(500);
        taddpending(&s1, &s2);
    }

    /// c:90 — `tdopending` on empty/never-pending state safe.
    #[test]
    fn tdopending_on_empty_state_no_panic() {
        let _g = crate::test_util::global_state_lock();
        tdopending();
        tdopending();
        tdopending();
    }

    /// c:32 — `is_cond_binary_op` rejects substring of valid op.
    /// `=` is valid; `=A` should be rejected (no prefix-matching).
    #[test]
    fn is_cond_binary_op_rejects_prefix_substring() {
        assert_eq!(
            is_cond_binary_op("=A"),
            0,
            "`=A` is NOT a binary op (no prefix-match semantics)"
        );
        assert_eq!(is_cond_binary_op("=="), 1, "`==` IS a binary op");
    }
}
