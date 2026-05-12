//! Canonical AST s-expression emitter for `ZshProgram`.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C
//! zsh has `Src/text.c` for AST→shell-source rendering (used by
//! `which -x`, `functions`, `whence -f`) but no S-expression
//! emitter. zshrs adds this canonical sexp form so the parity
//! harness in `tests/parity_harness.rs` can byte-compare zshrs's
//! parser output against the wordcode `zsh` writes via
//! `bin_zcompile()` (Src/parse.c → decoded by `zwc_decode`).
//!
//! Used by `tests/parity_harness.rs` to compare zshrs's parser
//! output against zsh's wordcode parser output (via `zwc_decode`).
//! Both sides emit IDENTICAL byte sequences for logically-
//! equivalent ASTs.
//!
//! ## Canonical sexp grammar
//!
//! ```text
//! program  := "(Program" (" " list)* ")"
//! list     := "(List " flags " " sublist ")"
//!             flags = "Sync" | "Async" | "Disown"
//! sublist  := "(Sublist " flags " " pipe (" " op " " sublist)? ")"
//!             flags = "()" | "(Not)" | "(Coproc)" | "(Not Coproc)"
//!             op    = "And" | "Or"
//! pipe     := "(Pipe " cmd (" " conn " " pipe)? ")"
//!             conn = "Pipe" | "PipeMerge"
//! cmd      := simple | subsh | cursh | for | case | if | while | until |
//!             repeat | funcdef | time | cond | arith | try | redirected
//! simple   := "(Simple " assigns " " words " " redirs ")"
//! assigns  := "(Assigns" (" " assign)* ")"
//! assign   := "(Set " STRING " " value ")" | "(Append " STRING " " value ")"
//! value    := "(Scalar " STRING ")" | "(Array" (" " STRING)* ")"
//! words    := "(Words" (" " STRING)* ")"
//! redirs   := "(Redirs" (" " redir)* ")"
//! redir    := "(" rtype " " FD " " STRING (" " VARID)? (" " heredoc)? ")"
//! heredoc  := "(Heredoc " TERM " " QUOTED_BOOL " " BODY ")"
//! ```
//!
//! Strings always emitted as `"..."` with `\\`, `\"`, `\n`, `\r`, `\t`,
//! and `\xNN` (for other control bytes) escaping. Empty containers
//! emitted as `(Tag)` with no inner space.
//!
//! Source-position info (line numbers, file paths) is INTENTIONALLY
//! excluded from the canonical form — zsh wordcode includes per-pipe
//! linenos but zshrs may not, and parity should not turn on positional
//! metadata.

use crate::parse::{
    CaseTerm, ForList, ListFlags, SublistOp, ZshAssign, ZshAssignValue, ZshCommand, ZshCond,
    ZshList, ZshPipe, ZshProgram, ZshRedir, ZshSimple, ZshSublist,
};
use crate::ported::zsh_h::{
    REDIR_APP, REDIR_APPNOW, REDIR_ERRAPP, REDIR_ERRAPPNOW, REDIR_ERRWRITE, REDIR_ERRWRITENOW,
    REDIR_HEREDOC, REDIR_HEREDOCDASH, REDIR_HERESTR, REDIR_INPIPE, REDIR_MERGEIN, REDIR_MERGEOUT,
    REDIR_OUTPIPE, REDIR_READ, REDIR_READWRITE, REDIR_WRITE, REDIR_WRITENOW,
};

/// Convert a parsed `ZshProgram` to canonical sexp.
/// zshrs-original — no C counterpart. The closest C analog is
/// `getpermtext()` from Src/text.c which renders the AST as zsh
/// shell source (not sexp).
pub fn ast_to_sexp(prog: &ZshProgram) -> String {
    let mut out = String::new();
    emit_program(prog, &mut out);
    out
}

fn emit_program(prog: &ZshProgram, out: &mut String) {
    out.push_str("(Program");
    for list in &prog.lists {
        out.push(' ');
        emit_list(list, out);
    }
    out.push(')');
}

fn emit_list(list: &ZshList, out: &mut String) {
    out.push_str("(List ");
    emit_list_flags(&list.flags, out);
    out.push(' ');
    emit_sublist(&list.sublist, out);
    out.push(')');
}

fn emit_list_flags(f: &ListFlags, out: &mut String) {
    let s = match (f.async_, f.disown) {
        (false, _) => "Sync",
        (true, false) => "Async",
        (true, true) => "Disown",
    };
    out.push_str(s);
}

fn emit_sublist(sl: &ZshSublist, out: &mut String) {
    out.push_str("(Sublist (");
    let mut first = true;
    if sl.flags.not {
        out.push_str("Not");
        first = false;
    }
    if sl.flags.coproc {
        if !first {
            out.push(' ');
        }
        out.push_str("Coproc");
    }
    out.push_str(") ");
    emit_pipe(&sl.pipe, out);
    if let Some((op, next)) = &sl.next {
        out.push(' ');
        out.push_str(match op {
            SublistOp::And => "And",
            SublistOp::Or => "Or",
        });
        out.push(' ');
        emit_sublist(next, out);
    }
    out.push(')');
}

fn emit_pipe(p: &ZshPipe, out: &mut String) {
    out.push_str("(Pipe ");
    emit_cmd(&p.cmd, out);
    if let Some(next) = &p.next {
        out.push(' ');
        out.push_str(if p.merge_stderr { "PipeMerge" } else { "Pipe" });
        out.push(' ');
        emit_pipe(next, out);
    }
    out.push(')');
}

fn emit_cmd(cmd: &ZshCommand, out: &mut String) {
    match cmd {
        ZshCommand::Simple(s) => emit_simple(s, out),
        ZshCommand::Subsh(p) => {
            out.push_str("(Subsh ");
            emit_program(p, out);
            out.push(')');
        }
        ZshCommand::Cursh(p) => {
            out.push_str("(Cursh ");
            emit_program(p, out);
            out.push(')');
        }
        ZshCommand::For(f) => {
            let tag = if f.is_select { "Select" } else { "For" };
            out.push('(');
            out.push_str(tag);
            out.push(' ');
            emit_str(&f.var, out);
            out.push(' ');
            match &f.list {
                ForList::Words(ws) => {
                    out.push_str("(Words");
                    for w in ws {
                        out.push(' ');
                        emit_str(w, out);
                    }
                    out.push(')');
                }
                ForList::CStyle { init, cond, step } => {
                    out.push_str("(CStyle ");
                    emit_str(init, out);
                    out.push(' ');
                    emit_str(cond, out);
                    out.push(' ');
                    emit_str(step, out);
                    out.push(')');
                }
                ForList::Positional => out.push_str("Positional"),
            }
            out.push(' ');
            emit_program(&f.body, out);
            out.push(')');
        }
        ZshCommand::Case(c) => {
            out.push_str("(Case ");
            emit_str(&c.word, out);
            for arm in &c.arms {
                out.push_str(" (Arm (");
                let mut first = true;
                for pat in &arm.patterns {
                    if !first {
                        out.push(' ');
                    }
                    emit_str(pat, out);
                    first = false;
                }
                out.push_str(") ");
                out.push_str(match arm.terminator {
                    CaseTerm::Break => "Break",
                    CaseTerm::Continue => "Continue",
                    CaseTerm::TestNext => "TestNext",
                });
                out.push(' ');
                emit_program(&arm.body, out);
                out.push(')');
            }
            out.push(')');
        }
        ZshCommand::If(i) => {
            out.push_str("(If ");
            emit_program(&i.cond, out);
            out.push(' ');
            emit_program(&i.then, out);
            for (cond, body) in &i.elif {
                out.push_str(" (Elif ");
                emit_program(cond, out);
                out.push(' ');
                emit_program(body, out);
                out.push(')');
            }
            if let Some(eb) = &i.else_ {
                out.push_str(" (Else ");
                emit_program(eb, out);
                out.push(')');
            }
            out.push(')');
        }
        ZshCommand::While(w) => {
            out.push_str("(While ");
            emit_program(&w.cond, out);
            out.push(' ');
            emit_program(&w.body, out);
            out.push(')');
        }
        ZshCommand::Until(w) => {
            out.push_str("(Until ");
            emit_program(&w.cond, out);
            out.push(' ');
            emit_program(&w.body, out);
            out.push(')');
        }
        ZshCommand::Repeat(r) => {
            out.push_str("(Repeat ");
            emit_str(&r.count, out);
            out.push(' ');
            emit_program(&r.body, out);
            out.push(')');
        }
        ZshCommand::FuncDef(f) => {
            out.push_str("(FuncDef (");
            let mut first = true;
            for n in &f.names {
                if !first {
                    out.push(' ');
                }
                emit_str(n, out);
                first = false;
            }
            out.push(')');
            if f.tracing {
                out.push_str(" (Tracing)");
            }
            if let Some(args) = &f.auto_call_args {
                out.push_str(" (AutoCallArgs");
                for a in args {
                    out.push(' ');
                    emit_str(a, out);
                }
                out.push(')');
            }
            out.push(' ');
            emit_program(&f.body, out);
            out.push(')');
        }
        ZshCommand::Time(opt) => {
            out.push_str("(Time");
            if let Some(sl) = opt {
                out.push(' ');
                emit_sublist(sl, out);
            }
            out.push(')');
        }
        ZshCommand::Cond(c) => {
            out.push_str("(Cond ");
            emit_cond(c, out);
            out.push(')');
        }
        ZshCommand::Arith(s) => {
            out.push_str("(Arith ");
            emit_str(s, out);
            out.push(')');
        }
        ZshCommand::Try(t) => {
            out.push_str("(Try ");
            emit_program(&t.try_block, out);
            out.push(' ');
            emit_program(&t.always, out);
            out.push(')');
        }
        ZshCommand::Redirected(inner, redirs) => {
            out.push_str("(Redirected ");
            emit_cmd(inner, out);
            out.push(' ');
            emit_redirs(redirs, out);
            out.push(')');
        }
    }
}

fn emit_cond(c: &ZshCond, out: &mut String) {
    match c {
        ZshCond::Not(inner) => {
            out.push_str("(Not ");
            emit_cond(inner, out);
            out.push(')');
        }
        ZshCond::And(a, b) => {
            out.push_str("(And ");
            emit_cond(a, out);
            out.push(' ');
            emit_cond(b, out);
            out.push(')');
        }
        ZshCond::Or(a, b) => {
            out.push_str("(Or ");
            emit_cond(a, out);
            out.push(' ');
            emit_cond(b, out);
            out.push(')');
        }
        ZshCond::Unary(op, x) => {
            out.push_str("(Unary ");
            emit_str(op, out);
            out.push(' ');
            emit_str(x, out);
            out.push(')');
        }
        ZshCond::Binary(x, op, y) => {
            out.push_str("(Binary ");
            emit_str(x, out);
            out.push(' ');
            emit_str(op, out);
            out.push(' ');
            emit_str(y, out);
            out.push(')');
        }
        ZshCond::Regex(s, p) => {
            out.push_str("(Regex ");
            emit_str(s, out);
            out.push(' ');
            emit_str(p, out);
            out.push(')');
        }
    }
}

fn emit_simple(s: &ZshSimple, out: &mut String) {
    out.push_str("(Simple (Assigns");
    for a in &s.assigns {
        out.push(' ');
        emit_assign(a, out);
    }
    out.push_str(") (Words");
    for w in &s.words {
        out.push(' ');
        emit_str(w, out);
    }
    out.push_str(") ");
    emit_redirs(&s.redirs, out);
    out.push(')');
}

fn emit_assign(a: &ZshAssign, out: &mut String) {
    out.push('(');
    out.push_str(if a.append { "Append " } else { "Set " });
    emit_str(&a.name, out);
    out.push(' ');
    match &a.value {
        ZshAssignValue::Scalar(s) => {
            out.push_str("(Scalar ");
            emit_str(s, out);
            out.push(')');
        }
        ZshAssignValue::Array(vs) => {
            out.push_str("(Array");
            for v in vs {
                out.push(' ');
                emit_str(v, out);
            }
            out.push(')');
        }
    }
    out.push(')');
}

fn emit_redirs(redirs: &[ZshRedir], out: &mut String) {
    out.push_str("(Redirs");
    for r in redirs {
        out.push(' ');
        emit_redir(r, out);
    }
    out.push(')');
}

fn emit_redir(r: &ZshRedir, out: &mut String) {
    out.push('(');
    out.push_str(redir_type_tag(r.rtype));
    out.push(' ');
    out.push_str(&r.fd.to_string());
    out.push(' ');
    emit_str(&r.name, out);
    if let Some(v) = &r.varid {
        out.push(' ');
        emit_str(v, out);
    }
    if let Some(h) = &r.heredoc {
        out.push_str(" (Heredoc ");
        emit_str(&h.terminator, out);
        out.push(' ');
        out.push_str(if h.quoted { "Quoted" } else { "Unquoted" });
        out.push(' ');
        emit_str(&h.content, out);
        out.push(')');
    }
    out.push(')');
}

fn redir_type_tag(t: i32) -> &'static str {
    match t {
        REDIR_WRITE => "Write",
        REDIR_WRITENOW => "Writenow",
        REDIR_APP => "Append",
        REDIR_APPNOW => "Appendnow",
        REDIR_READ => "Read",
        REDIR_READWRITE => "ReadWrite",
        REDIR_HEREDOC => "Heredoc",
        REDIR_HEREDOCDASH => "HeredocDash",
        REDIR_HERESTR => "Herestr",
        REDIR_MERGEIN => "MergeIn",
        REDIR_MERGEOUT => "MergeOut",
        REDIR_ERRWRITE => "ErrWrite",
        REDIR_ERRWRITENOW => "ErrWritenow",
        REDIR_ERRAPP => "ErrAppend",
        REDIR_ERRAPPNOW => "ErrAppendnow",
        REDIR_INPIPE => "InPipe",
        REDIR_OUTPIPE => "OutPipe",
        _ => "Unknown",
    }
}

fn emit_str(s: &str, out: &mut String) {
    // Our parser stores tokstr with zsh's internal token bytes (Dash=0x9b,
    // Star=0x87, Inpar=0x88, etc.) inline. zsh's wordcode side runs the
    // same strings through `untokenize` before AST emission (exec.c:2077).
    // Mirror that here so a `-` lexed as Dash inside a [[ ]] cond doesn't
    // serialize as raw \xc2\x9b in the parity sexp.
    let untok = crate::zwc::untokenize(s.as_bytes());
    out.push('"');
    for b in untok.bytes() {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(b as char),
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "\\x{:02x}", b);
            }
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
        
    fn parse_to_sexp(src: &str) -> String {
        use crate::ported::utils::{errflag, ERRFLAG_ERROR};
        use std::sync::atomic::Ordering;
        let saved = errflag.load(Ordering::Relaxed);
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        crate::ported::parse::parse_init(src);
        let prog = crate::ported::parse::parse();
        let had_err = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
        errflag.store(saved, Ordering::Relaxed);
        assert!(!had_err, "parse failed");
        ast_to_sexp(&prog)
    }

    #[test]
    fn simple_echo() {
        let s = parse_to_sexp("echo hello");
        assert!(s.starts_with("(Program (List Sync (Sublist ()"), "got: {}", s);
        assert!(s.contains(r#"(Words "echo" "hello")"#), "got: {}", s);
    }

    #[test]
    fn pipe_two_stages() {
        let s = parse_to_sexp("echo hi | grep h");
        assert!(s.contains("Pipe (Pipe"), "got: {}", s);
    }

    #[test]
    fn assign_then_cmd() {
        let s = parse_to_sexp("FOO=bar echo $FOO");
        assert!(s.contains(r#"(Set "FOO" (Scalar "bar"))"#), "got: {}", s);
    }

    #[test]
    fn andand() {
        let s = parse_to_sexp("true && echo yes");
        assert!(s.contains(" And "), "got: {}", s);
    }

    #[test]
    fn empty_program() {
        let s = parse_to_sexp("");
        assert_eq!(s, "(Program)");
    }
}
