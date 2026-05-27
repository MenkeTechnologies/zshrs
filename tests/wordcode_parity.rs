//! Wordcode parity harness: locks the Rust `par_*_wordcode` family
//! (parse.rs:1509+) byte-equal to C zsh's wordcode output.
//!
//! Why this exists: zshrs's production runtime IR is fusevm bytecode
//! (compile_zsh.rs walks the AST and emits typed Ops). zsh's runtime
//! IR is wordcode (a flat `u32[]` walked by exec.c). The two runtimes
//! diverge by design — that's the fusevm value-add.
//!
//! BUT the Rust port keeps a parallel `par_*_wordcode` implementation
//! that's a faithful port of C's parser-as-wordcode-emitter
//! (parse.c::par_event etc.). It currently emits stub wordcode and
//! isn't called from production. This harness measures how close
//! that side-port is to byte-equal with C, so future "C-faithful
//! runtime" work has a verified parser to feed.
//!
//! Reference data: `<basename>.wordcode` files alongside each corpus
//! entry, regenerated via `tests/lexer_corpus/regen.sh`.
//!
//! Format (must agree with both `zshrs_dump.c::bin_dumpwordcode` AND
//! `examples/parse_dump.rs`):
//!     EPROG flags=<hex> len=<int> npats=<int>
//!     WORDS <n>
//!     WC[i]=0x<hex> KIND=<name> DATA=0x<hex>     (n lines)
//!     STRS <n>
//!     STR[i]="<escaped>"                          (n lines)
//!
//! Initial expected state: ~0/44 — the `_wordcode` emitters are stubs.
//! As they're wired up, the score climbs.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use zsh::zsh_h::{wc_code, wc_data, wordcode};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lexer_corpus")
}

fn module_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/zsh/Src/Modules")
}

fn module_built() -> bool {
    let d = module_dir();
    d.join("zshrs_dump.so").exists() || d.join("zshrs_dump.bundle").exists()
}

fn zsh_available() -> bool {
    Command::new("zsh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn collect_corpus() -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(corpus_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    matches!(
                        p.extension().and_then(|s| s.to_str()),
                        Some("zsh") | Some("sh")
                    )
                })
                .filter(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .map(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    entries
}

fn shell_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn dump_via_zsh(file: &Path) -> Result<String, String> {
    let module_dir = module_dir().to_string_lossy().to_string();
    let script = format!(
        "module_path=({}); zmodload zsh/zshrs_dump; dumpwordcode {}",
        shell_escape(&module_dir),
        shell_escape(&file.to_string_lossy()),
    );
    let out = Command::new("zsh")
        .args(["-fc", &script])
        .output()
        .map_err(|e| format!("spawn zsh: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "zsh exit {:?}, stderr:\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn load_zsh_stream(path: &Path) -> Result<String, String> {
    let wc_path = path.with_file_name(format!(
        "{}.wordcode",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("?")
    ));
    if wc_path.exists() {
        return std::fs::read_to_string(&wc_path)
            .map_err(|e| format!("read {}: {}", wc_path.display(), e));
    }
    dump_via_zsh(path)
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

#[allow(dead_code)]
fn esc(out: &mut String, s: &str) {
    for b in s.bytes() {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            0 => out.push_str("\\0"),
            c if !(0x20..0x7f).contains(&c) => {
                let _ = write!(out, "\\x{:02x}", c);
            }
            c => out.push(c as char),
        }
    }
}

fn dump_via_zshrs(src: &str) -> String {
    use zsh::tokens::ENDINPUT;

    zsh::lex::lex_init(src);
    zsh::lex::set_tok(ENDINPUT);
    zsh::parse::init_parse();
    zsh::lex::zshlex();
    // Mirror C parse_list (parse.c:691-708) — par_list, not par_event.
    let mut cmplx: i32 = 0;
    zsh::parse::par_list_wordcode(&mut cmplx);
    if zsh::lex::tok() != ENDINPUT {
        // Match zsh's parse error format (zerrmsg in lex.c emits
        // "zsh:%d: parse error near `%s'" then PARSE_ERR). The
        // regen.sh corpus capture used `2>&1` so the stderr line
        // appears before PARSE_ERR.
        let line = zsh::lex::lineno();
        let tok_text = zsh::lex::tokstr().unwrap_or_else(|| "}".to_string());
        return format!("zsh:{}: parse error near `{}'\nPARSE_ERR\n", line, tok_text);
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
        // Match C dumpwordcode (zshrs_dump.c:308-313): calls
        // unmetafy(copy) BEFORE escaping for display. zsh stores
        // strs in METAFIED form; display shows unmetafied bytes.
        let _ = write!(buf, "STR[{}]=\"", i);
        let unmetafied = unmetafy_bytes(e);
        esc_bytes(&mut buf, &unmetafied);
        buf.push_str("\"\n");
    }
    buf
}

/// Unmetafy a byte slice in place: each `\x83 X` pair becomes
/// `X ^ 0x20`. Mirrors C `unmetafy` in Src/utils.c.
fn unmetafy_bytes(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if src[i] == 0x83 && i + 1 < src.len() {
            out.push(src[i + 1] ^ 0x20);
            i += 2;
        } else {
            out.push(src[i]);
            i += 1;
        }
    }
    out
}

fn esc_bytes(out: &mut String, bytes: &[u8]) {
    for &b in bytes {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            0 => out.push_str("\\0"),
            c if !(0x20..0x7f).contains(&c) => {
                let _ = write!(out, "\\x{:02x}", c);
            }
            c => out.push(c as char),
        }
    }
}

fn first_divergence(a: &str, b: &str) -> usize {
    a.bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()))
}

fn safe_slice(s: &str, lo: usize, hi: usize) -> &str {
    let start = (0..=lo.min(s.len()))
        .rev()
        .find(|i| s.is_char_boundary(*i))
        .unwrap_or(0);
    let end = (start..=hi.min(s.len()))
        .rev()
        .find(|i| s.is_char_boundary(*i))
        .unwrap_or(s.len());
    &s[start..end]
}

fn check_file(path: &Path) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;
    let zsh_stream = load_zsh_stream(path).map_err(|e| format!("zsh dump: {}", e))?;
    let zshrs_stream = dump_via_zshrs(&src);
    if zsh_stream == zshrs_stream {
        return Ok(());
    }
    let div = first_divergence(&zsh_stream, &zshrs_stream);
    let lo = div.saturating_sub(60);
    let hi_zsh = (div + 60).min(zsh_stream.len());
    let hi_zshrs = (div + 60).min(zshrs_stream.len());
    Err(format!(
        "\n--- {} ---\n\
         === first divergence at byte {} ===\n\
         zsh   ...{}...\n\
         zshrs ...{}...\n",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
        div,
        safe_slice(&zsh_stream, lo, hi_zsh).escape_default(),
        safe_slice(&zshrs_stream, lo, hi_zshrs).escape_default(),
    ))
}

#[test]
fn corpus_wordcode_parity() {
    let corpus = collect_corpus();
    if corpus.is_empty() {
        panic!("no corpus files in tests/lexer_corpus/");
    }
    let all_present = corpus.iter().all(|p| {
        p.with_file_name(format!(
            "{}.wordcode",
            p.file_name().and_then(|s| s.to_str()).unwrap_or("?")
        ))
        .exists()
    });
    if !all_present {
        if !zsh_available() {
            eprintln!("zsh not on PATH and not all .wordcode files present — skipping");
            return;
        }
        if !module_built() {
            eprintln!(
                "zsh/zshrs_dump module not built and not all .wordcode files present — \
                 skipping wordcode parity. Build the C module then run \
                 tests/lexer_corpus/regen.sh"
            );
            return;
        }
    }

    let mut passes = 0usize;
    let mut failures = Vec::new();
    for path in &corpus {
        match check_file(path) {
            Ok(()) => passes += 1,
            Err(report) => failures.push(report),
        }
    }
    eprintln!(
        "wordcode parity: {}/{} passing ({} failing)",
        passes,
        corpus.len(),
        failures.len()
    );
    if !failures.is_empty() {
        for f in failures.iter().take(3) {
            eprintln!("{}", f);
        }
        if failures.len() > 3 {
            eprintln!("... {} more failures elided ...", failures.len() - 3);
        }
        panic!(
            "wordcode parity FAILURES: {}/{}",
            failures.len(),
            corpus.len()
        );
    }
}
