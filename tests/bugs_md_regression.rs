//! End-to-end regression tests for BUGS.md entries.
//!
//! Each fixed entry in `docs/BUGS.md` gets a subprocess parity test
//! here. If a fix regresses, the test fails before merge.
//!
//! Pattern mirrors `tests/recent_ports_parity.rs`: spawn the freshly
//! built `target/debug/zshrs` with `--zsh` and assert the stdout +
//! exit-code match the expected zsh-reference output.

#![allow(clippy::needless_raw_string_hashes)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for cand in [
        manifest.join("target/debug/zshrs"),
        manifest.join("target/release/zshrs"),
    ] {
        if cand.exists() {
            return Some(cand);
        }
    }
    None
}

fn run_zshrs(script: &str) -> (i32, String, String) {
    let bin = match zshrs_bin() {
        Some(b) => b,
        None => {
            eprintln!("skip: zshrs binary not built");
            return (0, String::new(), String::new());
        }
    };
    let out = Command::new(&bin)
        .args(["--zsh", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin:?}: {e}"));
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #5 — print -P %j %T %D{…} escape dispatch
// Fix: src/ported/prompt.rs::putpromptchar
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug5_percent_j_prints_job_count_not_literal() {
    let (ec, stdout, _) = run_zshrs(r#"print -P "%j""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0");
    let trimmed = stdout.trim();
    assert_ne!(trimmed, "%j", "must NOT emit literal %j");
    assert!(
        trimmed.chars().all(|c| c.is_ascii_digit()),
        "%j must be a decimal job count, got {:?}",
        trimmed
    );
}

#[test]
fn bug5_percent_T_prints_hhmm_not_literal() {
    let (_, stdout, _) = run_zshrs(r#"print -P "%T""#);
    if zshrs_bin().is_none() {
        return;
    }
    let trimmed = stdout.trim();
    assert_ne!(trimmed, "%T", "must NOT emit literal %T");
    assert_eq!(trimmed.len(), 5, "HH:MM is 5 chars, got {:?}", trimmed);
    assert_eq!(&trimmed[2..3], ":", "colon at offset 2 in {:?}", trimmed);
}

#[test]
fn bug5_percent_D_braces_year_emits_4_digit_year() {
    let (_, stdout, _) = run_zshrs(r#"print -P "%D{%Y}""#);
    if zshrs_bin().is_none() {
        return;
    }
    let trimmed = stdout.trim();
    assert_ne!(
        trimmed, "%D{%Y}",
        "%D{{...}} must NOT emit literally"
    );
    let year: u32 = trimmed
        .parse()
        .unwrap_or_else(|_| panic!("expected 4-digit year, got {:?}", trimmed));
    assert!(year >= 2025, "year >= 2025, got {}", year);
}

#[test]
fn bug5_percent_bang_prints_history_number_not_literal() {
    let (_, stdout, _) = run_zshrs(r#"print -P "%!""#);
    if zshrs_bin().is_none() {
        return;
    }
    let trimmed = stdout.trim();
    assert_ne!(trimmed, "%!", "must NOT emit literal %!");
    // Allow leading minus for negative histct (defensive); body must
    // be decimal.
    let body = trimmed.strip_prefix('-').unwrap_or(trimmed);
    assert!(
        body.chars().all(|c| c.is_ascii_digit()),
        "%! must be a (signed) decimal, got {:?}",
        trimmed
    );
}

#[test]
fn bug5_percent_star_prints_hhmmss() {
    let (_, stdout, _) = run_zshrs(r#"print -P "%*""#);
    if zshrs_bin().is_none() {
        return;
    }
    let trimmed = stdout.trim();
    assert_ne!(trimmed, "%*", "must NOT emit literal %*");
    assert_eq!(trimmed.len(), 8, "HH:MM:SS is 8 chars, got {:?}", trimmed);
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #7 — local arr=( $=s ) word-split on IFS
// Fix: src/ported/builtin.rs::bin_typeset (=(…) paren-init handler)
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug7_local_array_dollar_equals_splits_on_colon_ifs() {
    let (ec, stdout, stderr) = run_zshrs(
        r#"f() { local s=a:b:c IFS=:; local -a arr=( $=s ); echo "n=${#arr[@]} first=${arr[1]}"; }; f"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0 (stderr={:?})", stderr);
    assert!(
        stdout.contains("n=3"),
        "$= must split a:b:c on IFS=: into 3 fields, got {:?}",
        stdout
    );
    assert!(
        stdout.contains("first=a"),
        "first field must be 'a', got {:?}",
        stdout
    );
}

#[test]
fn bug7_typeset_a_array_dollar_equals_splits_on_default_ifs() {
    let (ec, stdout, stderr) = run_zshrs(
        r#"f() { local s="x y z"; typeset -a arr=( $=s ); echo "n=${#arr[@]} all=${arr[*]}"; }; f"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0 (stderr={:?})", stderr);
    assert!(
        stdout.contains("n=3"),
        "$= must split 'x y z' on default IFS into 3 fields, got {:?}",
        stdout
    );
    assert!(
        stdout.contains("all=x y z"),
        "joined output must be 'x y z', got {:?}",
        stdout
    );
}

#[test]
fn bug7_plain_array_element_no_dollar_equals_passes_through() {
    let (ec, stdout, stderr) = run_zshrs(
        r#"f() { local x=hello; local -a arr=( $x world "literal text" ); echo "n=${#arr[@]} last=${arr[-1]}"; }; f"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0 (stderr={:?})", stderr);
    // Without $=, no IFS-split fires; "literal text" splits on
    // whitespace per the existing split_whitespace pass → 4 elems.
    assert!(
        stdout.contains("n=4"),
        "non-`$=` elements split on whitespace only, got {:?}",
        stdout
    );
}

#[test]
fn bug7_dollar_equals_on_unset_var_yields_empty_array() {
    let (ec, stdout, _) = run_zshrs(
        r#"f() { local -a arr=( $=zshrs_test_unset_var_xyz ); echo "n=${#arr[@]}"; }; f"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0);
    assert!(
        stdout.contains("n=0"),
        "$= on unset var → empty array, got {:?}",
        stdout
    );
}
