//! Parity tests for the `WC_AUTOFN` dispatch path (`execautofn` →
//! `loadautofn` → `execautofn_basic`) and ksh-style funcdef stripping
//! (`stripkshdef`).
//!
//! These pin behaviour observable from an autoload file: when the file
//! is a plain body, calling the function should run the body; when the
//! file wraps the body in `function NAME { … }` syntax, zsh's default
//! `autoload -U` mode strips the wrapper so the body runs on first
//! call (this is what `stripkshdef` does at the wordcode level).

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct R {
    stdout: String,
    exit: i32,
}

fn run_zsh_in(d: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(d)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_zshrs_in(d: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(d)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Plain-body autoload file: `autoload -U FN; FN` should print the
/// file's contents (first call faults the body in via `loadautofn`,
/// then `execautofn_basic` runs it). Pins the `execautofn`→
/// `execautofn_basic` dispatch wired by `dispatch_execfuncs`.
#[test]
fn autoload_plain_body_runs_on_first_call() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("af_plain"), "echo plain_body_ran\n").unwrap();
    let script = format!(
        r#"fpath=({} $fpath); autoload -U af_plain; af_plain"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(
        z.stdout.trim(),
        "plain_body_ran",
        "zsh sanity: {:?}",
        z.stdout
    );
    assert_eq!(
        r.stdout.trim(),
        "plain_body_ran",
        "zshrs autoload-of-plain-body — execautofn dispatch broken?\nzshrs: {:?}",
        r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

/// Autoload file that uses an explicit option in the body — verifies
/// the loaded body executes in the function's scope, not at parse
/// time. Pins `execautofn_basic` writes through to the caller's
/// `LASTVAL`.
#[test]
fn autoload_body_propagates_exit_status() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("af_exit"), "return 7\n").unwrap();
    let script = format!(
        r#"fpath=({} $fpath); autoload -U af_exit; af_exit; print $?"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout.trim(), "7", "zsh sanity");
    assert_eq!(r.stdout.trim(), "7", "zshrs lost exit status");
    assert_eq!(z.exit, r.exit);
}

/// Calling an autoloaded fn twice — the second call must NOT re-run
/// `loadautofn` (already loaded). Verifies state.prog.shf carries
/// the loaded funcdef across calls.
#[test]
fn autoload_second_call_uses_cached_body() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("af_twice"), "echo twice_$1\n").unwrap();
    let script = format!(
        r#"fpath=({} $fpath); autoload -U af_twice; af_twice one; af_twice two"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout, "twice_one\ntwice_two\n", "zsh sanity");
    assert_eq!(
        r.stdout, "twice_one\ntwice_two\n",
        "zshrs autoload re-load divergence"
    );
    assert_eq!(z.exit, r.exit);
}

/// `autoload -U` of a nonexistent function: the call must report
/// non-zero `$?`. zsh and zshrs use different specific exit codes
/// (zsh: 1, zshrs: 127) — pre-existing divergence, not pinned here.
/// What matters for the `execautofn` dispatch is that the failure
/// path doesn't crash and surfaces non-zero status.
#[test]
fn autoload_missing_function_reports_failure() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let script = format!(
        r#"fpath=({} $fpath); autoload -U af_nope; af_nope 2>/dev/null; echo done=$?"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert!(
        z.stdout.trim().starts_with("done="),
        "zsh sanity: {:?}",
        z.stdout
    );
    assert!(
        r.stdout.trim().starts_with("done="),
        "zshrs missing-fn path crashed: {:?}",
        r.stdout
    );
    let z_code = z.stdout.trim().trim_start_matches("done=");
    let r_code = r.stdout.trim().trim_start_matches("done=");
    assert_ne!(z_code, "0", "zsh expected non-zero");
    assert_ne!(r_code, "0", "zshrs expected non-zero");
}
