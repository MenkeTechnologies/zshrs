//! Source-to-IR dumpers exposed to the `zshrs` CLI as `--dump-tokens`,
//! `--dump-ast`, `--dump-wordcode`. Same logic as
//! `examples/{lex,ast,parse}_dump.rs` and the parity harnesses
//! (`tests/lexer_parity.rs`, `tests/parity_harness.rs`,
//! `tests/wordcode_parity.rs`) — keeping them in one library function
//! per IR so the binary, examples, and tests can all share output
//! format.
//!
//! Output formats match the C-side `zshrs_dump` builtins
//! (`dumptokens`, `dumpwordcode`) byte-for-byte, so output can be
//! diff'd against C zsh output for parity verification.

use std::fmt::Write as _;
use std::sync::atomic::Ordering;

use crate::lex;
use crate::parse;
use crate::tokens::{
    lextok, AMPER, AMPERBANG, AMPOUTANG, BANG_TOK, BARAMP, BAR_TOK, CASE, COPROC, DAMPER, DBAR,
    DINANG, DINANGDASH, DINBRACK, DINPAR, DOLOOP, DONE, DOUTANG, DOUTANGAMP, DOUTANGAMPBANG,
    DOUTANGBANG, DOUTBRACK, DOUTPAR, DSEMI, ELIF, ELSE, ENDINPUT, ENVARRAY, ENVSTRING, ESAC, FI,
    FOR, FOREACH, FUNC, IF, INANGAMP, INANG_TOK, INBRACE_TOK, INOUTANG, INOUTPAR, INPAR_TOK,
    LEXERR, NEWLIN, NOCORRECT, NULLTOK, OUTANGAMP, OUTANGAMPBANG, OUTANGBANG, OUTANG_TOK,
    OUTBRACE_TOK, OUTPAR_TOK, REPEAT, SELECT, SEMI, SEMIAMP, SEMIBAR, SEPER, STRING_LEX, THEN,
    TIME, TRINANG, TYPESET, UNTIL, WHILE, ZEND,
};
use crate::utils::{errflag, ERRFLAG_ERROR};
use crate::zsh_h::{wc_code, wc_data, wordcode};

/// Map a `lextok` to the canonical name used in the C-side
/// `zshrs_dump.c::toknames` table — kept in lockstep with
/// `tests/lexer_parity.rs::tok_name` so the same byte-equal harness
/// can consume CLI output.
fn tok_name(t: lextok) -> &'static str {
    match t {
        NULLTOK => "NULLTOK",
        SEPER => "SEPER",
        NEWLIN => "NEWLIN",
        SEMI => "SEMI",
        DSEMI => "DSEMI",
        AMPER => "AMPER",
        INPAR_TOK => "INPAR",
        OUTPAR_TOK => "OUTPAR",
        DBAR => "DBAR",
        DAMPER => "DAMPER",
        OUTANG_TOK => "OUTANG",
        OUTANGBANG => "OUTANGBANG",
        DOUTANG => "DOUTANG",
        DOUTANGBANG => "DOUTANGBANG",
        INANG_TOK => "INANG",
        INOUTANG => "INOUTANG",
        DINANG => "DINANG",
        DINANGDASH => "DINANGDASH",
        INANGAMP => "INANGAMP",
        OUTANGAMP => "OUTANGAMP",
        AMPOUTANG => "AMPOUTANG",
        OUTANGAMPBANG => "OUTANGAMPBANG",
        DOUTANGAMP => "DOUTANGAMP",
        DOUTANGAMPBANG => "DOUTANGAMPBANG",
        TRINANG => "TRINANG",
        BAR_TOK => "BAR",
        BARAMP => "BARAMP",
        INOUTPAR => "INOUTPAR",
        DINPAR => "DINPAR",
        DOUTPAR => "DOUTPAR",
        AMPERBANG => "AMPERBANG",
        SEMIAMP => "SEMIAMP",
        SEMIBAR => "SEMIBAR",
        DOUTBRACK => "DOUTBRACK",
        STRING_LEX => "STRING",
        ENVSTRING => "ENVSTRING",
        ENVARRAY => "ENVARRAY",
        ENDINPUT => "ENDINPUT",
        LEXERR => "LEXERR",
        BANG_TOK => "BANG",
        DINBRACK => "DINBRACK",
        INBRACE_TOK => "INBRACE",
        OUTBRACE_TOK => "OUTBRACE",
        CASE => "CASE",
        COPROC => "COPROC",
        DOLOOP => "DOLOOP",
        DONE => "DONE",
        ELIF => "ELIF",
        ELSE => "ELSE",
        ZEND => "ZEND",
        ESAC => "ESAC",
        FI => "FI",
        FOR => "FOR",
        FOREACH => "FOREACH",
        FUNC => "FUNC",
        IF => "IF",
        NOCORRECT => "NOCORRECT",
        REPEAT => "REPEAT",
        SELECT => "SELECT",
        THEN => "THEN",
        TIME => "TIME",
        UNTIL => "UNTIL",
        WHILE => "WHILE",
        TYPESET => "TYPESET",
        _ => "UNKNOWN",
    }
}

const WCNAMES: &[&str] = &[
    "WC_END",
    "WC_LIST",
    "WC_SUBLIST",
    "WC_PIPE",
    "WC_REDIR",
    "WC_ASSIGN",
    "WC_SIMPLE",
    "WC_TYPESET",
    "WC_SUBSH",
    "WC_CURSH",
    "WC_TIMED",
    "WC_FUNCDEF",
    "WC_FOR",
    "WC_SELECT",
    "WC_WHILE",
    "WC_REPEAT",
    "WC_CASE",
    "WC_IF",
    "WC_COND",
    "WC_ARITH",
    "WC_AUTOFN",
    "WC_TRY",
];

fn wc_name(kind: wordcode) -> &'static str {
    let i = kind as usize;
    if i < WCNAMES.len() {
        WCNAMES[i]
    } else {
        "WC_?"
    }
}

fn esc_bytes(out: &mut String, bytes: &[u8]) {
    for &b in bytes {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            0 => out.push_str("\\0"),
            c if c < 0x20 || c >= 0x7f => {
                let _ = write!(out, "\\x{:02x}", c);
            }
            c => out.push(c as char),
        }
    }
}

/// Drive the lexer over `src` and produce one `TOKNAME\tTOKSTR\n`
/// line per token (terminated by `ENDINPUT\n` or `LEXERR\n`). Same
/// format as the C-side `dumptokens` builtin in zshrs_dump.c.
pub fn dump_tokens(src: &str) -> String {
    let mut out = String::new();
    lex::lex_init(src);
    loop {
        lex::ctxtlex();
        let tok = lex::tok();
        if tok == ENDINPUT {
            out.push_str("ENDINPUT\n");
            return out;
        }
        if tok == LEXERR {
            out.push_str("LEXERR\n");
            return out;
        }
        let raw = lex::tokstr().unwrap_or_default();
        // For DISPLAY purposes (matching `Src/zsh dumptokens`
        // output byte-for-byte), `Qstring` (U+008C — the DQ-context
        // `$` marker per Src/zsh.h:167) must render as `$`.
        // `untokenize_preserve_quotes` deliberately leaves Qstring
        // raw so `stringsubst` at Src/subst.c:283 can branch on
        // `qt = c == Qstring`, but the lex-stream dump is consumed
        // by human eyeballs and diff harnesses against upstream's
        // dumptokens output, both of which expect `$`. Replace it
        // here after the untokenize pass so the in-tree consumer
        // contract (stringsubst etc.) is untouched but the dump
        // shows what `dumptokens` shows.
        let plain =
            lex::untokenize_preserve_quotes(&raw).replace(crate::ported::zsh_h::Qstring, "$");
        out.push_str(tok_name(tok));
        out.push('\t');
        out.push_str(&plain);
        out.push('\n');
    }
}

/// Parse `src` and dump the AST as canonical S-expression. Uses the
/// existing `ast_sexp::ast_to_sexp` so the output matches what the
/// AST parity harness (`tests/parity_harness.rs`) compares against
/// C's wordcode-decoded sexp.
pub fn dump_ast(src: &str) -> String {
    lex::lex_init(src);
    let saved = errflag.load(Ordering::Relaxed);
    errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
    let prog = parse::parse();
    let had_err = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
    errflag.store(saved, Ordering::Relaxed);
    if had_err {
        return "PARSE_ERR\n".to_string();
    }
    let mut out = crate::ast_sexp::ast_to_sexp(&prog);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Parse `src` via the wordcode emitter (`par_list_wordcode +
/// bld_eprog`) and dump the resulting Eprog in the canonical format
/// `EPROG / WORDS N / WC[i]=... / STRS N / STR[i]="..."`. Matches
/// the C-side `dumpwordcode` builtin byte-for-byte.
pub fn dump_wordcode(src: &str) -> String {
    lex::lex_init(src);
    lex::set_tok(ENDINPUT);
    parse::init_parse();
    lex::zshlex();
    let mut cmplx: i32 = 0;
    parse::par_list_wordcode(&mut cmplx);
    if lex::tok() != ENDINPUT {
        return "PARSE_ERR\n".to_string();
    }
    let prog = parse::bld_eprog(true);

    let mut buf = String::new();
    let _ = writeln!(
        buf,
        "EPROG flags=0x{:x} len={} npats={}",
        prog.flags, prog.len, prog.npats
    );
    let wc_count = prog.prog.len();
    let _ = writeln!(buf, "WORDS {}", wc_count);
    for (i, w) in prog.prog.iter().enumerate() {
        let _ = writeln!(
            buf,
            "WC[{}]=0x{:08x} KIND={} DATA=0x{:x}",
            i,
            w,
            wc_name(wc_code(*w)),
            wc_data(*w)
        );
    }
    let strs_str = prog.strs.unwrap_or_default();
    let strs_bytes = strs_str.as_bytes();
    let mut entries: Vec<&[u8]> = Vec::new();
    let mut start = 0;
    for (i, &b) in strs_bytes.iter().enumerate() {
        if b == 0 {
            entries.push(&strs_bytes[start..i]);
            start = i + 1;
        }
    }
    let _ = writeln!(buf, "STRS {}", entries.len());
    for (i, e) in entries.iter().enumerate() {
        let _ = write!(buf, "STR[{}]=\"", i);
        esc_bytes(&mut buf, e);
        buf.push_str("\"\n");
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::{esc_bytes, tok_name, wc_name};
    use crate::tokens::{
        AMPER, AMPERBANG, BAR_TOK, CASE, ENDINPUT, ESAC, FI, FOR, IF, LEXERR, NEWLIN, SEMI,
        STRING_LEX, THEN, WHILE,
    };

    // ─── tok_name: canonical-name lookup matches C-side toknames ──────

    #[test]
    fn tok_name_basic_separators() {
        assert_eq!(tok_name(NEWLIN), "NEWLIN");
        assert_eq!(tok_name(SEMI), "SEMI");
        assert_eq!(tok_name(AMPER), "AMPER");
    }

    #[test]
    fn tok_name_string_and_terminators() {
        assert_eq!(tok_name(STRING_LEX), "STRING");
        assert_eq!(tok_name(ENDINPUT), "ENDINPUT");
        assert_eq!(tok_name(LEXERR), "LEXERR");
    }

    #[test]
    fn tok_name_logical_ops() {
        assert_eq!(tok_name(BAR_TOK), "BAR");
        assert_eq!(tok_name(AMPERBANG), "AMPERBANG");
    }

    #[test]
    fn tok_name_control_keywords() {
        assert_eq!(tok_name(IF), "IF");
        assert_eq!(tok_name(THEN), "THEN");
        assert_eq!(tok_name(FI), "FI");
        assert_eq!(tok_name(FOR), "FOR");
        assert_eq!(tok_name(WHILE), "WHILE");
        assert_eq!(tok_name(CASE), "CASE");
        assert_eq!(tok_name(ESAC), "ESAC");
    }

    #[test]
    fn tok_name_unknown_falls_back_to_literal() {
        // Synthetic out-of-range lextok value should land in the wildcard arm.
        let bogus: super::lextok = i32::MAX;
        assert_eq!(tok_name(bogus), "UNKNOWN");
    }

    // ─── wc_name: WCNAMES table lookup with overflow guard ────────────

    #[test]
    fn wc_name_inside_range_returns_table_entry() {
        // kind=0 is always within the static WCNAMES table.
        assert!(!wc_name(0).is_empty(), "0 must hit the static table");
    }

    #[test]
    fn wc_name_overflow_falls_back() {
        // u32::MAX cast to usize indexes way past WCNAMES → "WC_?".
        assert_eq!(wc_name(u32::MAX), "WC_?");
    }

    // ─── esc_bytes: matches C-side zsh escape table ───────────────────

    fn esc(bytes: &[u8]) -> String {
        let mut s = String::new();
        esc_bytes(&mut s, bytes);
        s
    }

    #[test]
    fn esc_newline() {
        assert_eq!(esc(b"\n"), "\\n");
    }

    #[test]
    fn esc_tab() {
        assert_eq!(esc(b"\t"), "\\t");
    }

    #[test]
    fn esc_backslash_doubles() {
        assert_eq!(esc(b"\\"), "\\\\");
    }

    #[test]
    fn esc_double_quote() {
        assert_eq!(esc(b"\""), "\\\"");
    }

    #[test]
    fn esc_nul_byte() {
        assert_eq!(esc(b"\0"), "\\0");
    }

    #[test]
    fn esc_printable_ascii_passes_through() {
        // 0x20..=0x7e: passed through unmodified.
        assert_eq!(esc(b"hello world"), "hello world");
        assert_eq!(esc(b"!#$%&'()*+,-./0123456789"), "!#$%&'()*+,-./0123456789");
    }

    #[test]
    fn esc_low_control_uses_hex() {
        // 0x01..=0x1f (excluding the explicit cases above) → \xHH.
        assert_eq!(esc(&[0x01]), "\\x01");
        assert_eq!(esc(&[0x1f]), "\\x1f");
    }

    #[test]
    fn esc_high_bit_uses_hex() {
        // 0x7f and 0x80..=0xff → \xHH.
        assert_eq!(esc(&[0x7f]), "\\x7f");
        assert_eq!(esc(&[0xff]), "\\xff");
        assert_eq!(esc(&[0x80]), "\\x80");
    }

    #[test]
    fn esc_mixed_buffer_preserves_order() {
        assert_eq!(esc(b"a\nb\tc"), "a\\nb\\tc");
        assert_eq!(esc(b"x\0y"), "x\\0y");
        assert_eq!(esc(b"\"hi\""), "\\\"hi\\\"");
    }

    #[test]
    fn esc_appends_to_existing_buffer() {
        // esc_bytes is "write into provided String" — must append, not replace.
        let mut buf = String::from("prefix:");
        esc_bytes(&mut buf, b"\n");
        assert_eq!(buf, "prefix:\\n");
    }

    #[test]
    fn esc_empty_input_yields_empty_output() {
        assert_eq!(esc(b""), "");
    }
}
