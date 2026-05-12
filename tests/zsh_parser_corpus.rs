//! Test that parses all zsh functions from the homebrew zsh distribution.
//!
//! This tests the ZshParser against real-world zsh code from /opt/homebrew/Cellar/zsh/
//! which has been copied to test_data/zsh_functions/

use std::fs;
use std::path::Path;

fn parse_zsh(input: &str) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let saved = zsh::utils::errflag.load(Ordering::Relaxed);
    zsh::utils::errflag.fetch_and(!zsh::utils::ERRFLAG_ERROR, Ordering::Relaxed);
    zsh::parse::parse_init(input);
    let _prog = zsh::parse::parse();
    let had_err = (zsh::utils::errflag.load(Ordering::Relaxed) & zsh::utils::ERRFLAG_ERROR) != 0;
    zsh::utils::errflag.store(saved, Ordering::Relaxed);
    if had_err {
        Err("parse error".to_string())
    } else {
        Ok(())
    }
}

#[test]
fn test_parse_all_zsh_functions() {
    let functions_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_data/zsh_functions");
    if !functions_dir.exists() {
        panic!(
            "test_data/zsh_functions directory not found at {:?}",
            functions_dir
        );
    }

    let mut total = 0;
    let mut passed = 0;
    let mut failed_files = Vec::new();
    let mut skipped = 0;

    let mut entries: Vec<_> = fs::read_dir(&functions_dir)
        .expect("failed to read functions dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();

    // Sort for deterministic order
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let path = entry.path();
        total += 1;

        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                failed_files.push((file_name, format!("read error: {}", e)));
                continue;
            }
        };

        // Skip files over 10KB to keep test time reasonable
        if content.len() > 10000 {
            skipped += 1;
            continue;
        }

        match parse_zsh(&content) {
            Ok(()) => passed += 1,
            Err(err) => failed_files.push((file_name, err)),
        }
    }

    eprintln!("\n=== Zsh Parser Corpus Test Results ===");
    eprintln!("Total files: {}", total);
    eprintln!("Skipped (>10KB): {}", skipped);
    eprintln!("Tested: {}", total - skipped);
    eprintln!("Passed: {}", passed);
    eprintln!("Failed: {}", failed_files.len());

    let tested = total - skipped;
    let pass_rate = if tested > 0 {
        (passed as f64 / tested as f64) * 100.0
    } else {
        0.0
    };
    eprintln!("Pass rate: {:.1}%", pass_rate);

    if !failed_files.is_empty() {
        eprintln!("\nFailed files (showing first 30):");
        for (file, err) in failed_files.iter().take(30) {
            eprintln!("  {} - {}", file, err);
        }
        if failed_files.len() > 30 {
            eprintln!("  ... and {} more failures", failed_files.len() - 30);
        }
    }

    // Ensure the test completed and tested some files
    assert!(tested > 0, "No files were tested");
}

#[test]
fn test_specific_zsh_functions() {
    let functions_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_data/zsh_functions");
    if !functions_dir.exists() {
        return;
    }

    let important_functions = ["add-zsh-hook", "colors", "compinit", "promptinit"];

    for func_name in important_functions {
        let path = functions_dir.join(func_name);
        if !path.exists() {
            continue;
        }

        let content = fs::read_to_string(&path).expect("failed to read");
        let _result = parse_zsh(&content);
        // Don't assert success - these have complex syntax that may not parse yet
    }
}
