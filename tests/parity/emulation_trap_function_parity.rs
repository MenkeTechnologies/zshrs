//! `TRAPxxx()` FUNCTION TRAPS across emulation modes, against the REAL shells.
//!
//! zsh has two trap forms that are NOT interchangeable:
//!   * a LIST trap   — `trap 'cmd' INT`
//!   * a FUNCTION trap — `TRAPINT() { cmd }`
//! Setting one REMOVES the other for that signal, function traps show up in
//! the functions list rather than as list traps, and only list traps are
//! reset in a subshell.  All of that is zsh-specific.  bash has no such
//! concept at all: `TRAPINT` there is an ordinary function with an ordinary
//! name, it is never installed as a handler, and SIGINT keeps its default
//! disposition.
//!
//! !!! THE REFERENCE BINARY IS THE SPEC HERE — NOT zsh's C source !!!
//! Every expectation is produced by running the actual `bash` / `zsh` on this
//! machine, so nothing goes stale when a shell changes behaviour.
//!
//! A missing reference shell SKIPS its rows loudly and the test fails if NO
//! reference was found, so an absent binary can never quietly become a pass.

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

fn find_shell(candidates: &[&str]) -> Option<String> {
    for c in candidates {
        if c.starts_with('/') {
            if Path::new(c).exists() {
                return Some((*c).to_string());
            }
            continue;
        }
        if let Ok(out) = Command::new("/usr/bin/env")
            .args(["sh", "-c", &format!("command -v {c}")])
            .output()
        {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() && Path::new(&p).exists() {
                return Some(p);
            }
        }
    }
    None
}

#[derive(PartialEq)]
struct Out {
    stdout: String,
    exit: i32,
}

/// `exit` is -1 when the shell was KILLED BY A SIGNAL rather than exiting,
/// which is itself the observable under test for the SIGINT rows: bash dies,
/// and a correct `--bash` must die the same way instead of running a handler.
fn run(bin: &str, pre: &[&str], script: &str) -> Out {
    let o = Command::new(bin)
        .args(pre)
        .arg("-c")
        .arg(script)
        .env_remove("ZSHRS_CACHE")
        .env_remove("ENV")
        .env_remove("BASH_ENV")
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    Out {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// The four observables that separate the two trap forms.  `sleep 0.2` gives
/// an asynchronously-delivered signal time to land before the script ends.
const CASES: &[(&str, &str)] = &[
    (
        "fires-on-signal",
        r#"TRAPINT() { echo FIRED; }; kill -INT $$; sleep 0.2; echo end"#,
    ),
    (
        "trap-listing",
        r#"TRAPINT() { echo FIRED; }; trap"#,
    ),
    (
        "is-a-plain-function",
        r#"TRAPUSR1() { echo FIRED; }; kill -USR1 $$; sleep 0.2; echo end"#,
    ),
    (
        "mutual-exclusion-with-list-trap",
        r#"trap 'echo LIST' USR1; TRAPUSR1() { echo FN; }; kill -USR1 $$; sleep 0.2"#,
    ),
];

fn compare(family: &str, reference: &str, zshrs_pre: &[&str], ref_pre: &[&str]) {
    let z = zshrs_bin();
    let z = z.to_str().unwrap();
    let mut bad = Vec::new();
    for (name, script) in CASES {
        let want = run(reference, ref_pre, script);
        let got = run(z, zshrs_pre, script);
        if want != got {
            bad.push(format!(
                "  [{family}/{name}]\n    script : {script}\n    {reference}: stdout={:?} exit={}\n    zshrs  : stdout={:?} exit={}",
                want.stdout, want.exit, got.stdout, got.exit
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{family}: {} of {} TRAPxxx observables diverged from {reference}:\n{}",
        bad.len(),
        CASES.len(),
        bad.join("\n")
    );
}

/// NEGATIVE CONTROL, and the anti-regression half of the pair: `--zsh` must
/// keep firing function traps, keep listing them, and keep letting a function
/// trap DISPLACE an already-set list trap.  Fixing the `--bash` side must not
/// disturb any of this, so this row runs live rather than being pinned.
#[test]
fn zsh_mode_function_traps_match_real_zsh() {
    let Some(zsh) = find_shell(&["zsh", "/bin/zsh", "/usr/bin/zsh"]) else {
        eprintln!("SKIP: no zsh on this machine");
        return;
    };
    compare("zsh", &zsh, &["--zsh", "-f"], &["-f"]);
}

/// BUGS.md #1114 — `--bash` currently installs `TRAPINT` as a real SIGINT
/// handler, so `TRAPINT() { echo FIRED; }; kill -INT $$` prints `FIRED` and
/// the shell survives, where bash prints nothing and dies of SIGINT.  The
/// `TRAP*` recognition needs gating on `posix_faithful()` at
/// `src/ported/exec.rs:7361` and `src/fusevm_bridge.rs:11917`.
#[test]
#[ignore = "BUGS.md #1114 — TRAP* function traps are recognised in --bash, where bash has no such form"]
fn bash_mode_has_no_function_traps() {
    let Some(bash) = find_shell(&["bash", "/bin/bash", "/usr/local/bin/bash"]) else {
        eprintln!("SKIP: no bash on this machine");
        return;
    };
    compare("bash", &bash, &["--bash"], &[]);
}
