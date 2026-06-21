//! Lexer parity harness: lock `src/lex` to `src/zsh/Src/lex.c`.
//!
//! For each `tests/lexer_corpus/*.zsh`:
//!   1. Invoke `zsh` with the `zsh/zshrs_dump` module loaded and run
//!      `dumptokens FILE` — produces `TOKNAME\tTOKSTR\n` lines per token,
//!      terminated by `ENDINPUT\n` or `LEXERR\n`. Module source lives at
//!      `src/zsh/Src/Modules/zshrs_dump.{c,mdd}`. Build with
//!      `cd src/zsh && make Src/Modules/zshrs_dump.so` (or `.dylib` on
//!      macOS). The harness skips with a clear message if the module
//!      isn't built.
//!   2. Drive zshrs lex module over the same source bytes; collect
//!      `(lextok, tokstr)` pairs.
//!   3. Render both streams to identical canonical form
//!      (`TOKNAME\tTOKSTR\n` per token); diff byte-for-byte.
//!
//! Failures pinpoint exact lex.c → lex divergences with the
//! original input + both streams + first-divergence pointer.

use std::path::{Path, PathBuf};
use std::process::Command;

use zsh::tokens::{
    lextok, AMPER, AMPERBANG, AMPOUTANG, BANG_TOK, BARAMP, BAR_TOK, CASE, COPROC, DAMPER, DBAR,
    DINANG, DINANGDASH, DINBRACK, DINPAR, DOLOOP, DONE, DOUTANG, DOUTANGAMP, DOUTANGAMPBANG,
    DOUTANGBANG, DOUTBRACK, DOUTPAR, DSEMI, ELIF, ELSE, ENDINPUT, ENVARRAY, ENVSTRING, ESAC, FI,
    FOR, FOREACH, FUNC, IF, INANGAMP, INANG_TOK, INBRACE_TOK, INOUTANG, INOUTPAR, INPAR_TOK,
    LEXERR, NEWLIN, NOCORRECT, NULLTOK, OUTANGAMP, OUTANGAMPBANG, OUTANGBANG, OUTANG_TOK,
    OUTBRACE_TOK, OUTPAR_TOK, REPEAT, SELECT, SEMI, SEMIAMP, SEMIBAR, SEPER, STRING_LEX, THEN,
    TIME, TRINANG, TYPESET, UNTIL, WHILE, ZEND,
};

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
                // Corpus entries are numbered `NN_*.sh|zsh`; regen.sh
                // (the regenerator script) is the only non-numbered
                // *.sh in the dir.
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

/// Token name table — must match `enum lextok` in `zsh.h:304-336` in order.
/// Same table as `Src/Modules/zshrs_dump.c::toknames`.
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
/// and unmetafies tokstr before printing (see `Src/Modules/zshrs_dump.c`);
/// we do the equivalent here via `zsh::lexer::untokenize`.
fn dump_via_zshrs(src: &str) -> String {
    // Serialize in-process lexing against the other in-process
    // lex/parse consumers (wordcode_parity). The lexer mutates
    // process-global state (chline/chwords/chwordpos, history, typtab);
    // two test threads lexing concurrently corrupt it, surfacing as
    // spurious "parse error" / "array assignment exceeded maximum
    // elements" failures in whichever parse loses the race. Same guard
    // wordcode_parity takes.
    let _g = crate::parser_lock::parser_guard();
    let mut out = String::new();
    zsh::lex::lex_init(src);
    use zsh::tokens::{ENDINPUT, LEXERR};
    loop {
        // Match the C-side dump module which uses `ctxtlex` (lex.c:317),
        // NOT bare `zshlex` (lex.c:266). ctxtlex wraps zshlex with the
        // per-token incmdpos / inredir / oldpos / infor updates the
        // parser would otherwise apply between calls. zshlex alone
        // doesn't touch incmdpos (Phase 2 of the C-fidelity refactor
        // moved that logic out of zshlex to match C). Without ctxtlex
        // here, the Rust side would lex `{a,b,c}` as INBRACE+STRING
        // instead of one STRING, etc.
        zsh::lex::ctxtlex();
        let tok = zsh::lex::tok();
        if tok == ENDINPUT {
            out.push_str("ENDINPUT\n");
            return out;
        }
        if tok == LEXERR {
            out.push_str("LEXERR\n");
            return out;
        }
        let raw_owned = zsh::lex::tokstr().unwrap_or_default();
        let raw = raw_owned.as_str();
        // C zsh's `untokenize` (exec.c:2077-2099) maps SNULL → `'`, DNULL →
        // `"`, BNULL → `\`, and Qstring → `$` via the `ztokens` table
        // (lex.c:38). zshrs's `untokenize_preserve_quotes` matches C for
        // SNULL/DNULL/BNULL but PRESERVES Qstring (`\u{8c}`) so the
        // downstream `stringsubst` qt detection at Src/subst.c:283 fires
        // for DQ-context `$`. The parity test diffs against C zsh's
        // dumptokens (which runs untokenize), so decode Qstring → `$`
        // here in the test to match C exactly without losing the
        // Qstring marker for the in-pipeline subst path.
        let plain = zsh::lex::untokenize_preserve_quotes(raw).replace('\u{8c}', "$");
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

/// Load the C-side token stream for a corpus file. Prefers the
/// pre-generated `<basename>.tokens` file (committed to the repo so
/// parity runs without needing to build/run zsh's C lexer); falls
/// back to invoking zsh + the dump module when the file is missing
/// or outdated. Regenerate via `tests/lexer_corpus/regen.sh`.
fn load_zsh_stream(path: &Path) -> Result<String, String> {
    let tokens_path = path.with_file_name(format!(
        "{}.tokens",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("?")
    ));
    if tokens_path.exists() {
        return std::fs::read_to_string(&tokens_path)
            .map_err(|e| format!("read {}: {}", tokens_path.display(), e));
    }
    dump_via_zsh(path)
}

fn check_file(path: &Path) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;
    let zsh_stream = load_zsh_stream(path).map_err(|e| format!("zsh dump: {}", e))?;
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
    eprintln!("zsh   tokens: {} ({} bytes)", zsh_tokens, zsh_stream.len());
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
    let corpus = collect_corpus();
    // Pre-generated `.tokens` files (committed alongside each corpus
    // entry) are the C-side reference. If they're all present, parity
    // runs without needing zsh on PATH or the dump module built.
    let all_tokens_present = corpus.iter().all(|p| {
        p.with_file_name(format!(
            "{}.tokens",
            p.file_name().and_then(|s| s.to_str()).unwrap_or("?")
        ))
        .exists()
    });
    if !all_tokens_present {
        if !zsh_available() {
            eprintln!("zsh not on PATH and not all .tokens files present — skipping");
            return;
        }
        if !module_built() {
            eprintln!(
                "zsh/zshrs_dump module not built and not all .tokens files present — \
                 skipping lexer parity. Build with: cd src/zsh && make \
                 Src/Modules/zshrs_dump.so, then regen via \
                 tests/lexer_corpus/regen.sh"
            );
            return;
        }
    }
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
        panic!("lexer parity FAILURES: {}/{}", failures.len(), corpus.len());
    }
}
