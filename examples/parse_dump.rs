//! Rust-side wordcode dumper. Drives the wordcode-emitting parser
//! path (`par_event_wordcode` at parse.rs:1509 + `bld_eprog`) — the
//! same path C zsh uses — and prints the resulting `Eprog` in the
//! same canonical format as `zshrs_dump.c::bin_dumpwordcode`.
//!
//! Important: this is NOT the production Rust pipeline. Production
//! goes source → `parse()` → `ZshProgram` AST → `compile_zsh.rs` →
//! fusevm bytecode. The `par_*_wordcode` family is a parallel C-faithful
//! emitter kept around for parity testing — locking it byte-equal
//! to C's wordcode means future C-faithful runtime work has a
//! verified parser to feed.

use std::fmt::Write;

use zsh::zsh_h::{wc_code, wc_data, wordcode};

const WCNAMES: &[&str] = &[
    "WC_END",     "WC_LIST",    "WC_SUBLIST", "WC_PIPE",    "WC_REDIR",
    "WC_ASSIGN",  "WC_SIMPLE",  "WC_TYPESET", "WC_SUBSH",   "WC_CURSH",
    "WC_TIMED",   "WC_FUNCDEF", "WC_FOR",     "WC_SELECT",  "WC_WHILE",
    "WC_REPEAT",  "WC_CASE",    "WC_IF",      "WC_COND",    "WC_ARITH",
    "WC_AUTOFN",  "WC_TRY",
];

fn wc_name(kind: wordcode) -> &'static str {
    let i = kind as usize;
    if i < WCNAMES.len() { WCNAMES[i] } else { "WC_?" }
}

fn esc(out: &mut String, s: &str) {
    for b in s.bytes() {
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

fn main() {
    let path = std::env::args().nth(1).expect("usage: parse_dump FILE");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {}", path, e));

    zsh::lex::lex_init(&src);
    use zsh::tokens::ENDINPUT;
    zsh::lex::set_tok(ENDINPUT);
    zsh::parse::init_parse();
    zsh::lex::zshlex();

    // Mirror C parse_list (parse.c:691-708): call par_list, NOT
    // par_event. par_event has its own outer-WCB_END emission
    // (parse.rs:1524) which would duplicate the one bld_eprog adds
    // (parse.rs:5497), producing the wrong eprog len + extra END
    // words versus C.
    let mut cmplx: i32 = 0;
    zsh::parse::par_list_wordcode(&mut cmplx);

    if zsh::lex::tok() != ENDINPUT {
        // Emit zsh-compatible error format to stderr matching
        // `zerrmsg(stderr, "%s:%d: parse error near `%s'", prog, line, tok)`.
        // The lineno where lex stopped is zsh::lex::lineno(); the
        // failing token text is whatever's in tokstr or, for keyword
        // tokens, a reasonable name.
        let line = zsh::lex::lineno();
        let tok_text = zsh::lex::tokstr().unwrap_or_else(|| "}".to_string());
        eprintln!("zsh:{}: parse error near `{}'", line, tok_text);
        println!("PARSE_ERR");
        return;
    }
    let prog = zsh::parse::bld_eprog(true);

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
        // Match C `dumpwordcode` output. Strs holds unmetafied bytes;
        // C's dumpwordcode runs unmetafy as a precaution (no-op on
        // already-unmetafied data) then escapes byte-by-byte. Walk
        // raw bytes here and escape each — `String::from_utf8_lossy`
        // would replace single-byte zsh markers (e.g. Dash 0x9b)
        // with the U+FFFD replacement (`\xef\xbf\xbd`) breaking
        // byte-for-byte parity with C output.
        let _ = write!(buf, "STR[{}]=\"", i);
        esc_bytes(&mut buf, e);
        buf.push_str("\"\n");
    }
    print!("{}", buf);
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
