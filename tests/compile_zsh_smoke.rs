//! Smoke tests for the new ZshParser → ZshCompiler → fusevm::VM pipeline.
//!
//! These run the real zshrs binary with `ZSHRS_NEW_PIPELINE=1` to route
//! through the port AST end-to-end (no ShellParser involvement). As
//! `compile_zsh.rs` matures, more constructs migrate. Once parity is
//! reached with the corpus, ShellParser/ShellLexer/ShellCommand get
//! deleted and these become the canonical path.

use std::process::{Command, Stdio};
use std::time::Duration;

fn zshrs_bin() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target/debug/zshrs");
    p
}

fn run_via_zsh_pipeline(src: &str) -> (i32, String) {
    let mut child = Command::new(zshrs_bin())
        .args(["-f", "-c", src])
        .env("ZSHRS_NEW_PIPELINE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("zshrs binary missing — run `cargo build` first");

    let timeout = Duration::from_secs(8);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().unwrap();
                return (
                    status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&out.stdout).to_string(),
                );
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("zshrs hung on: {}", src);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("waitpid: {}", e),
        }
    }
}

#[test]
fn smoke_echo() {
    let (status, out) = run_via_zsh_pipeline("echo hi");
    assert_eq!(status, 0, "exit: {}", status);
    assert_eq!(out, "hi\n");
}

#[test]
fn smoke_echo_two_args() {
    let (status, out) = run_via_zsh_pipeline("echo a b");
    assert_eq!(status, 0);
    assert_eq!(out, "a b\n");
}

#[test]
fn smoke_assignment_then_echo() {
    let (status, out) = run_via_zsh_pipeline("x=42; echo $x");
    assert_eq!(status, 0);
    assert_eq!(out, "42\n");
}

#[test]
fn smoke_true_false_status() {
    let (status, _) = run_via_zsh_pipeline("true");
    assert_eq!(status, 0);
    let (status, _) = run_via_zsh_pipeline("false");
    assert_eq!(status, 1);
}

#[test]
fn smoke_and_or() {
    let (status, out) = run_via_zsh_pipeline("true && echo yes");
    assert_eq!(status, 0);
    assert_eq!(out, "yes\n");
}

#[test]
fn smoke_or_chain() {
    let (_, out) = run_via_zsh_pipeline("false || echo fallback");
    assert_eq!(out, "fallback\n");
}

#[test]
fn smoke_subshell() {
    let (status, out) = run_via_zsh_pipeline("(echo from-subshell)");
    assert_eq!(status, 0);
    assert_eq!(out, "from-subshell\n");
}

#[test]
fn smoke_brace_group() {
    let (status, out) = run_via_zsh_pipeline("{ echo a; echo b; }");
    assert_eq!(status, 0);
    assert_eq!(out, "a\nb\n");
}

#[test]
fn smoke_pipeline_simple() {
    let (status, out) = run_via_zsh_pipeline("echo hi | /bin/cat");
    assert_eq!(status, 0);
    assert_eq!(out, "hi\n");
}
