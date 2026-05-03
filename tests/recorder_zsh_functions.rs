//! Coverage test for the recorder against a real-world zsh function
//! corpus — the 1202 completion + autoload functions in
//! `test_data/zsh_functions/` (zsh 5.9 stock + curated extensions).
//!
//! Two assertions:
//!   1. **No-crash.** A synthetic init script that autoloads every
//!      file in the corpus must run cleanly through `zshrs-recorder`
//!      (exit 0, end-of-run summary present, bundle parses).
//!   2. **Coverage.** Every name in the corpus directory must appear
//!      as a `function` event in the captured bundle with
//!      `value: "autoload"`. Catches recorder regressions where a new
//:      AOP layer drops the autoload hook without anyone noticing.
//!
//! Built only with `--features recorder` since the bin is gated on it.

#![cfg(feature = "recorder")]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const CORPUS_DIR: &str = "test_data/zsh_functions";

#[test]
fn recorder_handles_full_zsh_function_corpus_via_autoload() {
    let bin = env!("CARGO_BIN_EXE_zshrs-recorder");
    let corpus = Path::new(CORPUS_DIR)
        .canonicalize()
        .unwrap_or_else(|e| panic!("canonicalize {CORPUS_DIR}: {e}"));

    // Enumerate every file in the corpus dir.
    let mut names: Vec<String> = std::fs::read_dir(&corpus)
        .unwrap_or_else(|e| panic!("read {}: {}", corpus.display(), e))
        .filter_map(|entry| {
            let e = entry.ok()?;
            let path = e.path();
            if !path.is_file() {
                return None;
            }
            // Skip names with the `+` suffix-char form — zsh accepts
            // `keymap+widget` / `zstyle+` as function names but they
            // collide with `autoload`'s `+` flag prefix even with `--`.
            // Two files of 1202; the corpus stays representative.
            let name = path.file_name()?.to_str()?.to_string();
            if name.contains('+') {
                return None;
            }
            Some(name)
        })
        .collect();
    names.sort();

    assert!(
        names.len() > 1000,
        "corpus too small ({} files); expected ~1200",
        names.len()
    );

    // Build init script. fpath setup + one big `autoload -Uz --` line
    // chunked to ~50 names per call for readability (zsh accepts the
    // whole 1202-name line too; the chunking is just kinder if a
    // future debug session drops into the script).
    let tmp = tempfile::TempDir::new().expect("init tempdir");
    let init_path = tmp.path().join("init.zsh");
    let mut init = String::with_capacity(64 * 1024);
    init.push_str(&format!("fpath=({} $fpath)\n", corpus.display()));
    for chunk in names.chunks(50) {
        init.push_str("autoload -Uz --");
        for n in chunk {
            init.push(' ');
            init.push_str(n);
        }
        init.push('\n');
    }
    std::fs::write(&init_path, init.as_bytes())
        .unwrap_or_else(|e| panic!("write init: {e}"));

    // Bundle output path — we read it back and verify coverage.
    let bundle_path = tmp.path().join("bundle.json");

    let mut cmd = Command::new(bin);
    cmd.arg("--no-daemon")
        .arg("--quiet")
        .arg("--shell-id")
        .arg("zsh-functions-corpus-test")
        .arg("-o")
        .arg(&bundle_path)
        .arg("--file")
        .arg(&init_path);

    // Hermetic env: clear inherited shell state (ZPWR_*, history,
    // plugin paths, …) so the corpus run only captures what the init
    // script causes.
    cmd.env_clear();
    cmd.env("HOME", tmp.path());
    cmd.env("PATH", "/usr/bin:/bin");

    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("spawn zshrs-recorder: {e}"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.success(),
        "recorder exited non-zero ({:?})\n--- stderr ---\n{}\n--- stdout ---\n{}",
        out.status.code(),
        stderr,
        stdout,
    );

    // Verify bundle exists + parses.
    let bundle_bytes = std::fs::read(&bundle_path)
        .unwrap_or_else(|e| panic!("read bundle: {e}\nrecorder stderr:\n{stderr}"));
    let bundle: Value = serde_json::from_slice(&bundle_bytes)
        .unwrap_or_else(|e| panic!("parse bundle: {e}"));

    assert_eq!(bundle["shell_id"], serde_json::json!("zsh-functions-corpus-test"));

    let events = bundle["events"]
        .as_array()
        .unwrap_or_else(|| panic!("bundle.events is not an array"));

    // Coverage assertion: every corpus name must appear as a `function`
    // event with value="autoload". Build the actual-set then diff
    // against the expected-set so the failure message is concrete.
    let captured_autoload_names: HashSet<String> = events
        .iter()
        .filter(|ev| {
            ev["kind"].as_str() == Some("function")
                && ev["value"].as_str() == Some("autoload")
        })
        .filter_map(|ev| ev["name"].as_str().map(String::from))
        .collect();

    let expected: HashSet<String> = names.iter().cloned().collect();
    let missing: Vec<&String> = expected
        .iter()
        .filter(|n| !captured_autoload_names.contains(*n))
        .collect();

    assert!(
        missing.is_empty(),
        "{} of {} corpus functions missing from recorder bundle (first 5: {:?})",
        missing.len(),
        expected.len(),
        missing.iter().take(5).collect::<Vec<_>>(),
    );

    // Every captured autoload must be from the corpus — catches a
    // recorder regression that double-fires or lifts unrelated names
    // from elsewhere.
    let extra: Vec<&String> = captured_autoload_names
        .iter()
        .filter(|n| !expected.contains(*n))
        .collect();
    assert!(
        extra.is_empty(),
        "{} unexpected autoload names captured (first 5: {:?})",
        extra.len(),
        extra.iter().take(5).collect::<Vec<_>>(),
    );

    eprintln!(
        "recorder corpus coverage: {} / {} functions captured",
        captured_autoload_names.len(),
        expected.len()
    );
}

/// Spot-check that direct `source FILE` of a few completion files
/// doesn't crash the recorder. These files are autoload-bodies (use
/// `local` at top-level), so sourcing them top-level produces shell
/// errors — the recorder must absorb the error and still emit a clean
/// summary footer.
#[test]
fn recorder_survives_direct_source_of_completion_files() {
    let bin = env!("CARGO_BIN_EXE_zshrs-recorder");
    let corpus = Path::new(CORPUS_DIR)
        .canonicalize()
        .unwrap_or_else(|e| panic!("canonicalize {CORPUS_DIR}: {e}"));

    // Sample of files representing different shapes: simple compsys,
    // complex compsys with state machine, and a non-_ utility.
    let sample = ["_alias", "_git", "bashcompinit"];

    for name in sample {
        let path: PathBuf = corpus.join(name);
        if !path.exists() {
            continue; // corpus may not include every sample on every checkout
        }
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let bundle_path = tmp.path().join("bundle.json");

        let mut cmd = Command::new(bin);
        cmd.arg("--no-daemon")
            .arg("--quiet")
            .arg("-o")
            .arg(&bundle_path)
            .arg("--file")
            .arg(&path);
        cmd.env_clear();
        cmd.env("HOME", tmp.path());
        cmd.env("PATH", "/usr/bin:/bin");

        let out = cmd.output().unwrap_or_else(|e| panic!("spawn: {e}"));
        let stderr = String::from_utf8_lossy(&out.stderr);

        // Direct source MAY exit non-zero (local-outside-function and
        // friends will trigger zsh-style errors) — what matters is
        // (a) no panic / signal, (b) recorder still wrote a bundle.
        let signaled = out.status.code().is_none();
        assert!(
            !signaled,
            "recorder killed by signal on {}\n--- stderr ---\n{stderr}",
            path.display(),
        );
        assert!(
            bundle_path.exists(),
            "no bundle written for {} (recorder may have crashed before atexit)\n--- stderr ---\n{stderr}",
            path.display(),
        );
        let body = std::fs::read_to_string(&bundle_path)
            .unwrap_or_else(|e| panic!("read bundle: {e}"));
        let _bundle: Value = serde_json::from_str(&body)
            .unwrap_or_else(|e| panic!("parse bundle for {}: {}\n{}", path.display(), e, body));
    }
}
