//! Smoke tests for the IDE-facing CLI flags:
//!   `zshrs --dump-reflection`  — JSON dump of builtins/keywords/options/special_vars
//!   `zshrs --docs NAME`        — markdown hover card for NAME
//!
//! These exist so a wrong-format regression in either flag is caught
//! before the IntelliJ plugin sees it (the tool window parses the JSON
//! directly; the docs popup expects clean markdown / non-zero exit on
//! missing names).

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn zshrs_binary() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

#[test]
fn dump_reflection_emits_valid_json_with_known_categories() {
    let out = Command::new(zshrs_binary())
        .arg("--dump-reflection")
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?} stderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let v: Value = serde_json::from_str(&stdout).expect("valid JSON");
    for cat in ["builtins", "keywords", "options", "special_vars"] {
        assert!(v[cat].is_object(), "category `{}` missing or not an object", cat);
        let m = v[cat].as_object().unwrap();
        assert!(!m.is_empty(), "category `{}` is empty", cat);
    }
    // Spot-check well-known entries the IntelliJ tool window relies on
    assert_eq!(v["builtins"]["cd"].as_str(), Some("builtin"));
    assert_eq!(v["keywords"]["if"].as_str(), Some("keyword"));
    assert_eq!(v["options"]["EXTENDED_GLOB"].as_str(), Some("option"));
    assert_eq!(v["special_vars"]["$?"].as_str(), Some("special"));
}

#[test]
fn docs_known_builtin_returns_markdown_card_and_exit_0() {
    let out = Command::new(zshrs_binary())
        .arg("--docs")
        .arg("cd")
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?} stderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("**cd**"), "no bold header: {}", s);
    assert!(s.contains("working directory"), "no description: {}", s);
}

#[test]
fn docs_known_keyword_returns_card() {
    let out = Command::new(zshrs_binary())
        .arg("--docs")
        .arg("if")
        .output()
        .expect("spawn");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("**if**"));
    assert!(s.contains("Conditional"));
}

#[test]
fn docs_known_special_var_returns_card() {
    let out = Command::new(zshrs_binary())
        .arg("--docs")
        .arg("$?")
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Exit status"), "got: {}", s);
}

#[test]
fn docs_unknown_name_exits_nonzero_with_stderr_message() {
    let out = Command::new(zshrs_binary())
        .arg("--docs")
        .arg("DEFINITELY_NOT_A_THING_zxqv")
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "expected nonzero exit");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no docs"), "stderr did not announce miss: {}", err);
}

#[test]
fn help_text_advertises_editor_integration_flags() {
    let out = Command::new(zshrs_binary())
        .arg("--help")
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("--lsp"), "--lsp missing from help");
    assert!(s.contains("--dap"), "--dap missing from help");
    assert!(s.contains("--dump-reflection"), "--dump-reflection missing from help");
    assert!(s.contains("--docs"), "--docs missing from help");
}
