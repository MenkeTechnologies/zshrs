//! AST parity harness: zsh wordcode AST vs zshrs parser AST.
//!
//! For each `.zsh` corpus file:
//! 1. Invoke `zsh -fc 'zcompile out.zwc input.zsh'` to produce wordcode.
//! 2. Decode the `.zwc` via `src/zwc_decode.rs::decode_zwc_first` and
//!    render to canonical sexp.
//! 3. Parse the same source via `crate::ported::parse::parse` and
//!    render to canonical sexp via `src/ast_sexp.rs::ast_to_sexp`.
//! 4. Assert byte-equal sexp strings.
//!
//! Mismatches produce the snippet, the zsh-side sexp, the zshrs-side sexp,
//! and a unified-diff-style first-divergence pointer for triage.
//!
//! Per CLAUDE.md endgame rule: this becomes part of the immune system
//! once green. Every new lex/parse port adds a corpus entry first,
//! confirms it fails, then fixes the port until it passes.

use std::path::{Path, PathBuf};
use std::process::Command;

use zsh::ast_sexp::ast_to_sexp;
use zsh::zwc_decode::decode_zwc_first;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity_corpus")
}

fn collect_corpus() -> Vec<PathBuf> {
    let dir = corpus_dir();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("zsh"))
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    entries
}

fn zsh_available() -> bool {
    Command::new("zsh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn compile_to_zwc(src_path: &Path, out_path: &Path) -> Result<(), String> {
    let status = Command::new("zsh")
        .args([
            "-fc",
            &format!(
                "zcompile {} {}",
                out_path.to_str().ok_or("non-utf8 out path")?,
                src_path.to_str().ok_or("non-utf8 in path")?,
            ),
        ])
        .status()
        .map_err(|e| format!("spawn zsh: {}", e))?;
    if !status.success() {
        return Err(format!("zcompile exit {:?}", status.code()));
    }
    if !out_path.exists() {
        return Err(format!("zcompile produced no output at {:?}", out_path));
    }
    Ok(())
}

fn first_divergence(a: &str, b: &str) -> usize {
    a.bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()))
}

fn report_mismatch(name: &str, src: &str, zsh_sexp: &str, zshrs_sexp: &str) -> String {
    let div = first_divergence(zsh_sexp, zshrs_sexp);
    let lo = div.saturating_sub(40);
    let hi_zsh = (div + 40).min(zsh_sexp.len());
    let hi_zshrs = (div + 40).min(zshrs_sexp.len());
    format!(
        "AST parity FAIL: {}\n\
         --- source ---\n{}\n\
         --- zsh   sexp ({} bytes) ---\n{}\n\
         --- zshrs sexp ({} bytes) ---\n{}\n\
         --- first divergence at byte {} ---\n\
         zsh   ...{}...\n\
         zshrs ...{}...\n",
        name,
        src.trim_end(),
        zsh_sexp.len(),
        zsh_sexp,
        zshrs_sexp.len(),
        zshrs_sexp,
        div,
        &zsh_sexp[lo..hi_zsh],
        &zshrs_sexp[lo..hi_zshrs],
    )
}

/// Run one parity check. Returns `Ok(())` on match, `Err(report)` on diff.
fn check_parity(src_path: &Path) -> Result<(), String> {
    let src = std::fs::read_to_string(src_path).map_err(|e| format!("read source: {}", e))?;
    let name = src_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("(unknown)")
        .to_string();

    // zsh side
    let tmp = tempfile::TempDir::new().map_err(|e| format!("tempdir: {}", e))?;
    let zwc = tmp.path().join("out.zwc");
    compile_to_zwc(src_path, &zwc).map_err(|e| format!("[{}] compile: {}", name, e))?;
    let prog = decode_zwc_first(&zwc)
        .map_err(|e| format!("[{}] decode: {}", name, e))?
        .ok_or_else(|| format!("[{}] zwc had no functions", name))?;
    let zsh_sexp = ast_to_sexp(&prog);

    // zshrs side
    zsh::parse::parse_init(&src);
    let zshrs_prog = zsh::parse::parse();
    let zshrs_sexp = ast_to_sexp(&zshrs_prog);

    if zsh_sexp == zshrs_sexp {
        Ok(())
    } else {
        Err(report_mismatch(&name, &src, &zsh_sexp, &zshrs_sexp))
    }
}

#[test]
fn corpus_parity() {
    if !zsh_available() {
        eprintln!("zsh not on PATH — skipping AST parity harness");
        return;
    }
    let corpus = collect_corpus();
    if corpus.is_empty() {
        panic!("no corpus files found in tests/parity_corpus/");
    }

    let mut passes = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for path in &corpus {
        match check_parity(path) {
            Ok(_) => passes += 1,
            Err(report) => failures.push(report),
        }
    }

    let total = corpus.len();
    eprintln!(
        "AST parity: {}/{} passing ({} failing)",
        passes,
        total,
        failures.len()
    );
    if !failures.is_empty() {
        for f in &failures {
            eprintln!("{}", f);
        }
        panic!(
            "AST parity FAILURES: {}/{} corpus entries diverged",
            failures.len(),
            total
        );
    }
}

/// Debug: single-file parity check for triage. Picks the smallest paired
/// real script in `~/.zpwr` and dumps full sexp on both sides. Prints to
/// stderr (run with `--nocapture`).
#[test]
#[ignore = "diagnostic only — run with `cargo test single_real -- --ignored --nocapture`"]
fn single_real() {
    let path =
        std::path::PathBuf::from(std::env::var("HOME").unwrap() + "/.zpwr/local/.tokens-post.sh");
    if !path.exists() {
        eprintln!("source not present, skipping");
        return;
    }
    let src = std::fs::read_to_string(&path).unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let zwc = tmp.path().join("out.zwc");
    let _ = Command::new("zsh")
        .args([
            "-fc",
            &format!("zcompile {} {}", zwc.display(), path.display()),
        ])
        .status();
    let prog = decode_zwc_first(&zwc).unwrap().unwrap();
    let zsh_sexp = ast_to_sexp(&prog);
    let zshrs_prog = {
        zsh::parse::parse_init(&src);
        zsh::parse::parse()
    };
    let zshrs_sexp = ast_to_sexp(&zshrs_prog);
    eprintln!("=== source ({} bytes) ===\n{}", src.len(), src);
    eprintln!(
        "=== zsh   sexp ({} bytes) ===\n{}",
        zsh_sexp.len(),
        zsh_sexp
    );
    eprintln!(
        "=== zshrs sexp ({} bytes) ===\n{}",
        zshrs_sexp.len(),
        zshrs_sexp
    );
    let div = zsh_sexp
        .bytes()
        .zip(zshrs_sexp.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or(zsh_sexp.len().min(zshrs_sexp.len()));
    eprintln!("=== first divergence at byte {} ===", div);
    let lo = div.saturating_sub(80);
    let hi_zsh = (div + 80).min(zsh_sexp.len());
    let hi_zshrs = (div + 80).min(zshrs_sexp.len());
    eprintln!("zsh   ...{}...", &zsh_sexp[lo..hi_zsh]);
    eprintln!("zshrs ...{}...", &zshrs_sexp[lo..hi_zshrs]);
}

/// Real-world coverage: any paired source/`.zwc` files in `~/.zpwr` get
/// audited. The .zwc was produced by zsh during normal use; we re-parse
/// the source with zshrs and compare. Skipped when the zpwr tree isn't
/// present (CI / fresh checkouts).
#[test]
fn zpwr_real_world_parity() {
    if !zsh_available() {
        eprintln!("zsh not on PATH — skipping real-world parity");
        return;
    }
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };
    let zpwr = PathBuf::from(home).join(".zpwr");
    if !zpwr.exists() {
        eprintln!("~/.zpwr not present — skipping real-world parity");
        return;
    }

    // Walk recursively, find paired (foo, foo.zwc).
    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    #[allow(clippy::only_used_in_recursion)]
    fn walk(dir: &Path, pairs: &mut Vec<(PathBuf, PathBuf)>, depth: usize) {
        if depth > 4 {
            return;
        }
        let rd = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return,
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, pairs, depth + 1);
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("zwc") {
                continue;
            }
            // Pair with non-.zwc twin.
            let s = p.to_string_lossy();
            if let Some(stripped) = s.strip_suffix(".zwc") {
                let src = PathBuf::from(stripped);
                if src.exists() {
                    pairs.push((src, p));
                }
            }
        }
    }
    walk(&zpwr, &mut pairs, 0);

    if pairs.is_empty() {
        eprintln!("no paired .zwc files found in ~/.zpwr — skipping");
        return;
    }

    let mut passes = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (src_path, _zwc_path) in &pairs {
        // Re-compile via zsh into a fresh tmp .zwc — using the on-disk .zwc
        // would require it to have been compiled by the same zsh version.
        // Re-compile gives us a known reference.
        match check_parity(src_path) {
            Ok(_) => passes += 1,
            Err(report) => failures.push(report),
        }
    }

    eprintln!(
        "~/.zpwr real-world parity: {}/{} passing",
        passes,
        pairs.len()
    );
    // Real-world entries are EXPECTED to fail until lex/parse gaps are
    // closed. Don't fail the test on these; treat as informational until
    // we have the ground-truth gap list. Once the corpus baseline is
    // green, this test gets converted to assert hard.
    if !failures.is_empty() {
        eprintln!(
            "(informational) {}/{} real-world entries diverged",
            failures.len(),
            pairs.len()
        );
        // Print first 3 failure reports so we can iterate on real divergences.
        for f in failures.iter().take(3) {
            eprintln!("\n========\n{}", f);
        }
    }
}
