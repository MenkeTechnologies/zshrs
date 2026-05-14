//! Port of `Src/text.c` — textual representations of wordcode (`gettext2`),
//! permanent text (`getpermtext`), job text (`getjobtext`), redirection text
//! (`getredirs`), and the shared character-buffer helpers (`tadd*`, etc.).

use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::lex;
use crate::parse;
use crate::ported::linklist::LinkList;
use crate::ported::mem::{queue_signals, unqueue_signals};
use crate::ported::utils::{has_token, quotestring};
use crate::ported::zsh_h;
use crate::ported::zsh_h::{
    redir, wordcode, estate, Eprog, EC_NODUP, COND_AND, COND_MOD, COND_MODI, COND_NOT, COND_OR,
    COND_STREQ, COND_STRDEQ, COND_STRNEQ, IS_READFD, JOBTEXTSIZE, META, REDIRF_FROM_HEREDOC,
    REDIR_APP, REDIR_APPNOW, REDIR_ERRAPP, REDIR_ERRAPPNOW, REDIR_ERRWRITE, REDIR_ERRWRITENOW,
    REDIR_HEREDOC, REDIR_HERESTR, REDIR_INPIPE, REDIR_MERGEIN, REDIR_MERGEOUT, REDIR_OUTPIPE,
    REDIR_READ, REDIR_READWRITE, REDIR_WRITE, REDIR_WRITENOW, Z_ASYNC, Z_DISOWN, Z_END, Z_SIMPLE,
    WC_ARITH, WC_ASSIGN, WC_AUTOFN, WC_CASE, WC_CASE_AND, WC_CASE_OR, WC_COND, WC_COUNT, WC_CURSH, WC_END,
    WC_FOR, WC_FUNCDEF, WC_IF, WC_IF_ELIF, WC_LIST, WC_PIPE, WC_PIPE_END, WC_PIPE_MID, WC_REPEAT,
    WC_REDIR, WC_SELECT, WC_SELECT_LIST, WC_SIMPLE, WC_SUBLIST, WC_SUBLIST_SKIP, WC_SUBSH, WC_TIMED,
    WC_TIMED_PIPE, WC_TRY, WC_TYPESET, WC_WHILE, WC_ASSIGN_ARRAY, WC_CASE_SKIP, WC_CASE_TYPE,
    WC_COND_SKIP, WC_COND_TYPE, WC_CURSH_SKIP, WC_FOR_COND, WC_FOR_LIST, WC_FOR_TYPE,
    WC_FUNCDEF_SKIP, WC_IF_SKIP, WC_IF_TYPE, WC_LIST_TYPE, WC_PIPE_TYPE, WC_SELECT_TYPE,
    WC_SIMPLE_ARGC, WC_SUBLIST_COPROC, WC_SUBLIST_END, WC_SUBLIST_FLAGS, WC_SUBLIST_NOT,
    WC_SUBLIST_OR, WC_SUBLIST_SIMPLE, WC_SUBLIST_TYPE, WC_SUBSH_SKIP, WC_TIMED_TYPE, WC_TYPESET_ARGC,
    WC_WHILE_TYPE, WC_WHILE_UNTIL, wc_code,
};

/// Port of `is_cond_binary_op(const char *str)` from `Src/text.c:58`.
pub fn is_cond_binary_op(str: &str) -> i32 {
    COND_BINARY_OPS.iter().any(|&op| op == str) as i32
}

/// Port of `dec_tindent` from `Src/text.c:70`.
pub fn dec_tindent() {
    tindent.with(|t| {
        let mut v = t.borrow_mut();
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
    let mut state = estate {
        prog,
        pc: c.unwrap_or(0),
        strs: None,
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
    lex::untokenize(&raw)
}

/// Port of `getjobtext(Eprog prog, Wordcode c)` from `Src/text.c:315`.
pub fn getjobtext(prog: Eprog, c: Option<usize>) -> String {
    queue_signals();
    useeprog(&prog);
    let mut state = estate {
        prog,
        pc: c.unwrap_or(0),
        strs: None,
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
    if raw.ends_with(META) {
        raw.pop();
    }
    let p = state.prog;
    freeeprog(&p);
    unqueue_signals();
    lex::untokenize(&raw)
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
                        pop: if WC_SUBLIST_TYPE(code) == WC_SUBLIST_END { 1 } else { 0 },
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
                            let sk = if WC_SUBLIST_SKIP(fr.code) == 0 {
                                1
                            } else {
                                0
                            };
                            let p = state.pc;
                            let pipe_chk = if p < state.prog.prog.len() {
                                wc_code(state.prog.prog[p])
                            } else {
                                WC_COUNT
                            };
                            let not_simple_pipe =
                                (WC_SUBLIST_FLAGS(fr.code) as i32 & WC_SUBLIST_SIMPLE as i32)
                                    == 0
                                    && pipe_chk != WC_PIPE;
                            taddstr(if sk != 0 || not_simple_pipe { "!" } else { "! " });
                            stack = sk;
                        }
                        if (WC_SUBLIST_FLAGS(fr.code) as i32 & WC_SUBLIST_COPROC as i32) != 0
                        {
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
                        pop: if WC_PIPE_TYPE(code) == WC_PIPE_END { 1 } else { 0 },
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
                                    if ctype == COND_STREQ || ctype == COND_STRDEQ || ctype == COND_STRNEQ {
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
                    let d = u8::try_from(f.fd1).unwrap_or(b'0');
                    taddchr(i32::from(b'0' + (d % 10)));
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
                            taddstr(&quotestring(&n, crate::ported::zsh_h::QT_SINGLE));
                            tpush(b'\'' as i32);
                        } else {
                            tpush(b'"' as i32);
                            taddstr(&quotestring(&n, crate::ported::zsh_h::QT_DOUBLE));
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
            _ => {}
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

// c:1022-1026 — fstr[] (getredirs)
const FSTR: [&str; 18] = [
    ">", ">|", ">>", ">>|", "&>", "&>|", "&>>", "&>>|", "<>", "<", "<<", "<<-", "<<", "<&", ">&",
    ">&-",
    "<", ">",
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
    fn is_cond_binary_matches_zsh_set() {
        assert_eq!(is_cond_binary_op("="), 1);
        assert_eq!(is_cond_binary_op("-eq"), 1);
        assert_eq!(is_cond_binary_op("-nt"), 1);
        assert_eq!(is_cond_binary_op("-f"), 0);
        assert_eq!(is_cond_binary_op("foo"), 0);
    }

    #[test]
    fn zoutputtab_honours_text_expand_tabs() {
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
}
