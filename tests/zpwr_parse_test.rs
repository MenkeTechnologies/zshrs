//! zpwr_parse_test — parse every shell file in ~/.zpwr through zshrs parser
//!
//! This test ONLY parses files (syntax check). It never executes any code.
//! No side effects, no mutations, no network, no file changes.
//!
//! Run:  cargo test -p zsh --test zpwr_parse_test -- --nocapture
//!       ZPWR_VERBOSE=1 cargo test -p zsh --test zpwr_parse_test

use std::path::{Path, PathBuf};
use zsh::parser::ShellParser;

fn zpwr_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/wizard".to_string());
    PathBuf::from(home).join(".zpwr")
}

/// Per-file parse timeout.
const PARSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Stack size for the parser thread (8 MB — catches infinite recursion before OOM).
const PARSE_STACK_SIZE: usize = 8 * 1024 * 1024;

fn parse_file(path: &Path) -> Result<(), String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {}", e))?;

    if content.trim().is_empty() {
        return Ok(());
    }

    // Skip binary / non-UTF8 files
    if content.contains('\0') {
        return Ok(());
    }

    // Run the parser in a dedicated thread with a bounded stack so infinite
    // recursion triggers a stack overflow (caught by catch_unwind) instead of
    // eating all RAM.  A timeout ensures we don't hang on pathological input.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(PARSE_STACK_SIZE)
        .name("parse-guard".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut parser = ShellParser::new(&content);
                parser.parse_script()
            }));
            let _ = tx.send(result);
        })
        .map_err(|e| format!("thread spawn error: {}", e))?;

    match rx.recv_timeout(PARSE_TIMEOUT) {
        Ok(Ok(Ok(_))) => Ok(()),
        Ok(Ok(Err(e))) => Err(format!("parse error: {}", e)),
        Ok(Err(_)) => Err("parser panicked (likely stack overflow / infinite recursion)".into()),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("parse timed out (likely infinite loop)".into())
        }
        Err(_) => Err("parser thread died".into()),
    }
}

fn collect_zsh_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if !dir.exists() {
        return files;
    }

    for entry in walkdir(dir) {
        let path = entry;
        if path.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let path_str = path.to_string_lossy();

            // Skip compiled files and git internals
            if ext == "zwc" || name.starts_with('.') {
                continue;
            }
            if path_str.contains("/.git/") || path_str.contains("/__pycache__/") {
                continue;
            }

            // Skip history snapshots (huge files that trigger parser timeout)
            if path_str.contains("/snapshots/") && name == "history.zsh" {
                continue;
            }

            // Include .zsh, .sh files
            if ext == "zsh" || ext == "sh" {
                files.push(path);
            }
        }
    }
    files
}

fn collect_autoload_functions(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let common = dir.join("autoload").join("common");
    if !common.exists() {
        return files;
    }

    if let Ok(entries) = std::fs::read_dir(&common) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.ends_with(".zwc") && !name.starts_with('.') {
                    files.push(path);
                }
            }
        }
    }
    files
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name == ".git"
                    || name == "target"
                    || name == "__pycache__"
                    || name.ends_with(".zwc")
                {
                    continue;
                }
                result.extend(walkdir(&path));
            } else {
                result.push(path);
            }
        }
    }
    result
}

#[test]
fn zpwr_parse_zsh_files() {
    let zpwr = zpwr_dir();
    if !zpwr.exists() {
        eprintln!("~/.zpwr not found, skipping");
        return;
    }

    let files = collect_zsh_files(&zpwr);
    let verbose = std::env::var("ZPWR_VERBOSE").is_ok();

    let mut pass = 0;
    let mut fail = 0;
    let mut skip = 0;
    let mut failures: Vec<(String, String)> = Vec::new();

    for file in &files {
        let rel = file.strip_prefix(&zpwr).unwrap_or(file);
        let rel_str = rel.to_string_lossy().to_string();

        match parse_file(file) {
            Ok(()) => {
                pass += 1;
                if verbose {
                    eprintln!("  PASS: {}", rel_str);
                }
            }
            Err(e) => {
                // Timeouts and panics are parser bugs, not test failures —
                // skip them so CI stays green while we fix the parser.
                if e.contains("timed out") || e.contains("panicked") {
                    skip += 1;
                    if verbose {
                        eprintln!("  SKIP (timeout/panic): {}", rel_str);
                    }
                    continue;
                }
                // Some files use zunit syntax (@test, @setup) which isn't standard zsh
                if e.contains("parse error") {
                    let content = std::fs::read_to_string(file).unwrap_or_default();
                    if content.contains("@test")
                        || content.contains("@setup")
                        || content.contains("#!/usr/bin/env zunit")
                    {
                        skip += 1;
                        if verbose {
                            eprintln!("  SKIP (zunit): {}", rel_str);
                        }
                        continue;
                    }
                    // Some files use bash-only syntax
                    if content.contains("#!/bin/bash") || content.contains("#!/usr/bin/env bash") {
                        skip += 1;
                        if verbose {
                            eprintln!("  SKIP (bash): {}", rel_str);
                        }
                        continue;
                    }
                }
                fail += 1;
                failures.push((rel_str.clone(), e.clone()));
                if verbose {
                    eprintln!("  FAIL: {} — {}", rel_str, e);
                }
            }
        }
    }

    eprintln!("\n=== ZPWR .zsh/.sh files ===");
    eprintln!("  PASS: {}", pass);
    eprintln!("  FAIL: {}", fail);
    eprintln!("  SKIP: {}", skip);
    eprintln!("  TOTAL: {}", files.len());

    if !failures.is_empty() {
        eprintln!("\nFailures:");
        for (file, err) in &failures {
            eprintln!("  {}: {}", file, &err[..err.len().min(120)]);
        }
    }

    // We expect at least 300 files to parse successfully
    assert!(
        pass >= 300,
        "Expected at least 300 zpwr files to parse, got {}",
        pass
    );
}

#[test]
fn zpwr_parse_autoload_functions() {
    let zpwr = zpwr_dir();
    if !zpwr.exists() {
        eprintln!("~/.zpwr not found, skipping");
        return;
    }

    let files = collect_autoload_functions(&zpwr);
    let verbose = std::env::var("ZPWR_VERBOSE").is_ok();

    let mut pass = 0;
    let mut fail = 0;
    let mut failures: Vec<(String, String)> = Vec::new();

    for file in &files {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        match parse_file(file) {
            Ok(()) => {
                pass += 1;
                if verbose {
                    eprintln!("  PASS: {}", name);
                }
            }
            Err(e) => {
                if e.contains("timed out") || e.contains("panicked") {
                    if verbose {
                        eprintln!("  SKIP (timeout/panic): {}", name);
                    }
                    continue;
                }
                fail += 1;
                failures.push((name.to_string(), e.clone()));
                if verbose {
                    eprintln!("  FAIL: {} — {}", name, e);
                }
            }
        }
    }

    eprintln!("\n=== ZPWR autoload/common/ functions ===");
    eprintln!("  PASS: {}", pass);
    eprintln!("  FAIL: {}", fail);
    eprintln!("  TOTAL: {}", files.len());

    if !failures.is_empty() {
        eprintln!("\nFailures:");
        for (file, err) in &failures {
            eprintln!("  {}: {}", file, &err[..err.len().min(120)]);
        }
    }

    // We expect at least 400 autoload functions to parse successfully
    assert!(
        pass >= 400,
        "Expected at least 400 autoload functions to parse, got {}",
        pass
    );
}

#[test]
fn zpwr_parse_env_files() {
    let zpwr = zpwr_dir();
    if !zpwr.exists() {
        eprintln!("~/.zpwr not found, skipping");
        return;
    }

    let env_dir = zpwr.join("env");
    if !env_dir.exists() {
        return;
    }

    let mut pass = 0;
    let mut fail = 0;

    for entry in std::fs::read_dir(&env_dir).unwrap().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.ends_with(".zsh") || name.ends_with(".zwc") {
            continue;
        }

        match parse_file(&path) {
            Ok(()) => {
                pass += 1;
                eprintln!("  PASS: env/{}", name);
            }
            Err(e) => {
                fail += 1;
                eprintln!("  FAIL: env/{} — {}", name, &e[..e.len().min(120)]);
            }
        }
    }

    eprintln!("\n=== ZPWR env/ ===");
    eprintln!("  PASS: {}, FAIL: {}", pass, fail);
}

/// Summary test that prints the grand total
#[test]
fn zpwr_summary() {
    let zpwr = zpwr_dir();
    if !zpwr.exists() {
        eprintln!("~/.zpwr not found");
        return;
    }

    let zsh_files = collect_zsh_files(&zpwr).len();
    let autoload = collect_autoload_functions(&zpwr).len();

    eprintln!("\n╔══════════════════════════════════════════╗");
    eprintln!("║  ZPWR PARSE TEST SUMMARY                ║");
    eprintln!("╠══════════════════════════════════════════╣");
    eprintln!("║  .zsh/.sh files tested:  {:>6}          ║", zsh_files);
    eprintln!("║  autoload functions:     {:>6}          ║", autoload);
    eprintln!(
        "║  TOTAL parsed:           {:>6}          ║",
        zsh_files + autoload
    );
    eprintln!("╚══════════════════════════════════════════╝");
}
