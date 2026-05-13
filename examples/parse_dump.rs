//! Rust-side parser dumper. Same output format as the C-side
//! `dumpwordcode` builtin in `zshrs_dump.c` so the parser parity
//! harness can diff byte-for-byte.
//!
//! Format:
//!   EPROG flags=<hex> len=<int> npats=<int>
//!   WORDS <n>
//!   WC[i]=0x<hex> KIND=<name> DATA=0x<hex>     (n lines)
//!   STRS <n>
//!   STR[i]="<escaped>"                          (n lines)

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

fn print_escaped(s: &str) {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            0 => out.push_str("\\0"),
            c if c < 0x20 || c >= 0x7f => out.push_str(&format!("\\x{:02x}", c)),
            c => out.push(c as char),
        }
    }
    print!("{}", out);
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: parse_dump FILE");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {}", path, e));
    zsh::lex::lex_init(&src);

    let prog = match zsh::parse::parse_list() {
        Some(p) => p,
        None => {
            println!("PARSE_ERR");
            return;
        }
    };

    // Header.
    println!(
        "EPROG flags=0x{:x} len={} npats={}",
        prog.flags, prog.len, prog.npats
    );

    let wc_count = prog.prog.len();
    println!("WORDS {}", wc_count);
    for (i, w) in prog.prog.iter().enumerate() {
        println!(
            "WC[{}]=0x{:08x} KIND={} DATA=0x{:x}",
            i,
            w,
            wc_name(wc_code(*w)),
            wc_data(*w)
        );
    }

    // Strs table — Vec<u8>-like concat of \0-separated entries.
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
    // Trailing partial slot without terminator — skip (matches C
    // dumpwordcode behavior which also rejects unterminated tail).
    println!("STRS {}", entries.len());
    for (i, e) in entries.iter().enumerate() {
        let as_str = String::from_utf8_lossy(e);
        let plain = zsh::lex::untokenize(&as_str);
        print!("STR[{}]=\"", i);
        print_escaped(&plain);
        println!("\"");
    }
}
