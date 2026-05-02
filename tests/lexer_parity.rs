//! Lexer parity harness: lock `src/lexer.rs` to `src/zsh/Src/lex.c`.
//!
//! For each `tests/lexer_corpus/*.zsh`:
//!   1. Invoke `zsh` with the `zsh/zshrs_dump` module loaded and run
//!      `dumptokens FILE` — produces `TOKNAME\tTOKSTR\n` lines per token,
//!      terminated by `ENDINPUT\n` or `LEXERR\n`. Module source lives at
//!      `src/zsh/Src/Modules/zshrs_dump.{c,mdd}`. Build with
//!      `cd src/zsh && make Src/Modules/zshrs_dump.so` (or `.dylib` on
//!      macOS). The harness skips with a clear message if the module
//!      isn't built.
//!   2. Drive zshrs's `ZshLexer` over the same source bytes; collect
//!      `(LexTok, tokstr)` pairs.
//!   3. Render both streams to identical canonical form
//!      (`TOKNAME\tTOKSTR\n` per token); diff byte-for-byte.
//!
//! Failures pinpoint exact lex.c → lexer.rs divergences with the
//! original input + both streams + first-divergence pointer.

use std::path::{Path, PathBuf};
use std::process::Command;

use zsh::lexer::ZshLexer;
use zsh::tokens::LexTok;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lexer_corpus")
}

fn module_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/zsh/Src/Modules")
}

fn module_built() -> bool {
    let d = module_dir();
    d.join("zshrs_dump.so").exists() || d.join("zshrs_dump.dylib").exists()
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
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    entries
}

/// Token name table — must match `enum lextok` in `zsh.h:304-336` in order.
/// Same table as `Src/Modules/zshrs_dump.c::toknames`.
fn tok_name(t: LexTok) -> &'static str {
    match t {
        LexTok::Nulltok => "NULLTOK",
        LexTok::Seper => "SEPER",
        LexTok::Newlin => "NEWLIN",
        LexTok::Semi => "SEMI",
        LexTok::Dsemi => "DSEMI",
        LexTok::Amper => "AMPER",
        LexTok::Inpar => "INPAR",
        LexTok::Outpar => "OUTPAR",
        LexTok::Dbar => "DBAR",
        LexTok::Damper => "DAMPER",
        LexTok::Outang => "OUTANG",
        LexTok::Outangbang => "OUTANGBANG",
        LexTok::Doutang => "DOUTANG",
        LexTok::Doutangbang => "DOUTANGBANG",
        LexTok::Inang => "INANG",
        LexTok::Inoutang => "INOUTANG",
        LexTok::Dinang => "DINANG",
        LexTok::Dinangdash => "DINANGDASH",
        LexTok::Inangamp => "INANGAMP",
        LexTok::Outangamp => "OUTANGAMP",
        LexTok::Ampoutang => "AMPOUTANG",
        LexTok::Outangampbang => "OUTANGAMPBANG",
        LexTok::Doutangamp => "DOUTANGAMP",
        LexTok::Doutangampbang => "DOUTANGAMPBANG",
        LexTok::Trinang => "TRINANG",
        LexTok::Bar => "BAR",
        LexTok::Baramp => "BARAMP",
        LexTok::Inoutpar => "INOUTPAR",
        LexTok::Dinpar => "DINPAR",
        LexTok::Doutpar => "DOUTPAR",
        LexTok::Amperbang => "AMPERBANG",
        LexTok::Semiamp => "SEMIAMP",
        LexTok::Semibar => "SEMIBAR",
        LexTok::Doutbrack => "DOUTBRACK",
        LexTok::String => "STRING",
        LexTok::Envstring => "ENVSTRING",
        LexTok::Envarray => "ENVARRAY",
        LexTok::Endinput => "ENDINPUT",
        LexTok::Lexerr => "LEXERR",
        LexTok::Bang => "BANG",
        LexTok::Dinbrack => "DINBRACK",
        LexTok::Inbrace => "INBRACE",
        LexTok::Outbrace => "OUTBRACE",
        LexTok::Case => "CASE",
        LexTok::Coproc => "COPROC",
        LexTok::Doloop => "DOLOOP",
        LexTok::Done => "DONE",
        LexTok::Elif => "ELIF",
        LexTok::Else => "ELSE",
        LexTok::Zend => "ZEND",
        LexTok::Esac => "ESAC",
        LexTok::Fi => "FI",
        LexTok::For => "FOR",
        LexTok::Foreach => "FOREACH",
        LexTok::Func => "FUNC",
        LexTok::If => "IF",
        LexTok::Nocorrect => "NOCORRECT",
        LexTok::Repeat => "REPEAT",
        LexTok::Select => "SELECT",
        LexTok::Then => "THEN",
        LexTok::Time => "TIME",
        LexTok::Until => "UNTIL",
        LexTok::While => "WHILE",
        LexTok::Typeset => "TYPESET",
    }
}

fn local_zsh() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/zsh/Src/zsh")
}

/// Run dumptokens via the zsh subprocess. Returns raw stdout (token stream).
fn dump_via_zsh(file: &Path) -> Result<String, String> {
    let module_dir = module_dir().to_string_lossy().to_string();
    // `module_path=(...)` points zsh at our unbuilt-tree module dir; `-fc`
    // prevents user .zshenv/.zshrc from running. `zmodload` then loads the
    // dumptokens builtin and we invoke it on the file.
    let script = format!(
        "module_path=({}); zmodload zsh/zshrs_dump; dumptokens {}",
        shell_escape(&module_dir),
        shell_escape(&file.to_string_lossy()),
    );
    let zsh_bin = local_zsh();
    let zsh_cmd: &Path = if zsh_bin.exists() {
        zsh_bin.as_path()
    } else {
        Path::new("zsh")
    };
    let out = Command::new(zsh_cmd)
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

/// Drive zshrs's lexer over the source string, formatting each token as
/// `TOKNAME\tTOKSTR\n`, terminated by `ENDINPUT\n` or `LEXERR\n` to match
/// the dumptokens module output exactly. The C-side module untokenizes
/// + unmetafies tokstr before printing (see `Src/Modules/zshrs_dump.c`);
/// we do the equivalent here via `zsh::lexer::untokenize`.
fn dump_via_zshrs(src: &str) -> String {
    let mut out = String::new();
    let mut lex = ZshLexer::new(src);
    loop {
        lex.zshlex();
        let tok = lex.tok;
        if tok == LexTok::Endinput {
            out.push_str("ENDINPUT\n");
            return out;
        }
        if tok == LexTok::Lexerr {
            out.push_str("LEXERR\n");
            return out;
        }
        let raw = lex.tokstr.as_deref().unwrap_or("");
        // C zsh's `untokenize` (exec.c:2077-2099) maps SNULL → `'`, DNULL →
        // `"`, BNULL → `\` via the `ztokens` table (lex.c:38). zshrs's
        // plain `untokenize` strips them; `untokenize_preserve_quotes`
        // matches C behavior. Use the latter for parity.
        let plain = zsh::lexer::untokenize_preserve_quotes(raw);
        out.push_str(tok_name(tok));
        out.push('\t');
        out.push_str(&plain);
        out.push('\n');
    }
}

fn first_divergence(a: &str, b: &str) -> usize {
    a.bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()))
}

fn check_file(path: &Path) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;
    let zsh_stream = dump_via_zsh(path).map_err(|e| format!("zsh dump: {}", e))?;
    let zshrs_stream = dump_via_zshrs(&src);
    if zsh_stream == zshrs_stream {
        return Ok(());
    }
    let div = first_divergence(&zsh_stream, &zshrs_stream);
    let lo = div.saturating_sub(40);
    let hi_zsh = (div + 40).min(zsh_stream.len());
    let hi_zshrs = (div + 40).min(zshrs_stream.len());
    Err(format!(
        "\n--- {} ---\n\
         === source ===\n{}\n\
         === zsh   stream ({} bytes) ===\n{}\
         === zshrs stream ({} bytes) ===\n{}\
         === first divergence at byte {} ===\n\
         zsh   ...{}...\n\
         zshrs ...{}...\n",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
        src.trim_end(),
        zsh_stream.len(),
        zsh_stream,
        zshrs_stream.len(),
        zshrs_stream,
        div,
        safe_slice(&zsh_stream, lo, hi_zsh).escape_default(),
        safe_slice(&zshrs_stream, lo, hi_zshrs).escape_default(),
    ))
}

/// Slice a string by byte indices, snapping each end to the nearest
/// preceding char boundary so UTF-8 multi-byte sequences aren't split.
fn safe_slice(s: &str, lo: usize, hi: usize) -> &str {
    let start = (0..=lo.min(s.len())).rev().find(|i| s.is_char_boundary(*i)).unwrap_or(0);
    let end = (start..=hi.min(s.len())).rev().find(|i| s.is_char_boundary(*i)).unwrap_or(s.len());
    &s[start..end]
}

/// Diagnostic: run lexer parity against `~/.zshrc`. Prints stats + first
/// divergence to stderr. Does not fail the suite — informational like
/// `parity_harness::zpwr_real_world_parity`.
#[test]
#[ignore = "diagnostic — run with `cargo test zshrc_lex -- --ignored --nocapture`"]
fn zshrc_lex() {
    if !zsh_available() || !module_built() {
        eprintln!("zsh or module not available — skipping");
        return;
    }
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };
    let path = PathBuf::from(home).join(".zshrc");
    if !path.exists() {
        eprintln!("~/.zshrc not present, skipping");
        return;
    }
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read .zshrc: {}", e);
            return;
        }
    };
    let zsh_stream = match dump_via_zsh(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zsh dump: {}", e);
            return;
        }
    };
    let zshrs_stream = dump_via_zshrs(&src);

    let zsh_tokens = zsh_stream.lines().count();
    let zshrs_tokens = zshrs_stream.lines().count();
    eprintln!("~/.zshrc bytes: {}", src.len());
    eprintln!(
        "zsh   tokens: {} ({} bytes)",
        zsh_tokens,
        zsh_stream.len()
    );
    eprintln!(
        "zshrs tokens: {} ({} bytes)",
        zshrs_tokens,
        zshrs_stream.len()
    );

    if zsh_stream == zshrs_stream {
        eprintln!("~/.zshrc parity: byte-equal");
        return;
    }

    // Find first diverging line + show 2 lines of context on each side.
    let mut zsh_lines = zsh_stream.lines();
    let mut zshrs_lines = zshrs_stream.lines();
    let mut line_no = 0usize;
    loop {
        line_no += 1;
        let z = zsh_lines.next();
        let r = zshrs_lines.next();
        match (z, r) {
            (None, None) => break,
            (Some(a), Some(b)) if a == b => continue,
            (a, b) => {
                eprintln!("first divergence at token line {}:", line_no);
                eprintln!("  zsh   = {:?}", a.unwrap_or("<EOF>"));
                eprintln!("  zshrs = {:?}", b.unwrap_or("<EOF>"));
                // Show 3 prior lines for context.
                let ctx_start = line_no.saturating_sub(3);
                let zsh_ctx: Vec<&str> = zsh_stream
                    .lines()
                    .skip(ctx_start)
                    .take(line_no - ctx_start)
                    .collect();
                let zshrs_ctx: Vec<&str> = zshrs_stream
                    .lines()
                    .skip(ctx_start)
                    .take(line_no - ctx_start)
                    .collect();
                eprintln!(
                    "context (last {} matching lines):\n{}",
                    zsh_ctx.len().saturating_sub(1),
                    zsh_ctx[..zsh_ctx.len().saturating_sub(1)].join("\n")
                );
                let _ = zshrs_ctx;
                break;
            }
        }
    }
}

#[test]
fn corpus_lexer_parity() {
    if !zsh_available() {
        eprintln!("zsh not on PATH — skipping lexer parity");
        return;
    }
    if !module_built() {
        eprintln!(
            "zsh/zshrs_dump module not built — skipping lexer parity.\n\
             Build with: cd src/zsh && make Src/Modules/zshrs_dump.so"
        );
        return;
    }
    let corpus = collect_corpus();
    if corpus.is_empty() {
        panic!("no corpus files in tests/lexer_corpus/");
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
        "lexer parity: {}/{} passing ({} failing)",
        passes,
        corpus.len(),
        failures.len()
    );
    if !failures.is_empty() {
        for f in &failures {
            eprintln!("{}", f);
        }
        panic!(
            "lexer parity FAILURES: {}/{}",
            failures.len(),
            corpus.len()
        );
    }
}
