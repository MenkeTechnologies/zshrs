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

// ── Control flow ──

#[test]
fn smoke_if_then() {
    let (status, out) = run_via_zsh_pipeline("if true; then echo yes; fi");
    assert_eq!(status, 0);
    assert_eq!(out, "yes\n");
}

#[test]
fn smoke_if_else() {
    let (status, out) = run_via_zsh_pipeline("if false; then echo no; else echo yes; fi");
    assert_eq!(status, 0);
    assert_eq!(out, "yes\n");
}

#[test]
fn smoke_if_elif() {
    let (status, out) =
        run_via_zsh_pipeline("if false; then echo a; elif true; then echo b; else echo c; fi");
    assert_eq!(status, 0);
    assert_eq!(out, "b\n");
}

#[test]
fn smoke_while() {
    let (status, out) = run_via_zsh_pipeline(
        "i=0; while (( i < 3 )); do echo $i; (( i++ )); done",
    );
    assert_eq!(status, 0);
    assert_eq!(out, "0\n1\n2\n");
}

#[test]
fn smoke_until() {
    let (status, out) = run_via_zsh_pipeline(
        "i=0; until (( i >= 3 )); do echo $i; (( i++ )); done",
    );
    assert_eq!(status, 0);
    assert_eq!(out, "0\n1\n2\n");
}

#[test]
fn smoke_for_in_words() {
    let (status, out) = run_via_zsh_pipeline("for x in a b c; do echo $x; done");
    assert_eq!(status, 0);
    assert_eq!(out, "a\nb\nc\n");
}

#[test]
fn smoke_for_arith() {
    let (status, out) = run_via_zsh_pipeline("for ((i=0; i<3; i++)); do echo $i; done");
    assert_eq!(status, 0);
    assert_eq!(out, "0\n1\n2\n");
}

#[test]
fn smoke_case_match() {
    let (status, out) =
        run_via_zsh_pipeline("case x in a) echo a ;; x) echo x ;; *) echo o ;; esac");
    assert_eq!(status, 0);
    assert_eq!(out, "x\n");
}

#[test]
fn smoke_case_default() {
    let (status, out) =
        run_via_zsh_pipeline("case foo in a) echo a ;; *) echo def ;; esac");
    assert_eq!(status, 0);
    assert_eq!(out, "def\n");
}

#[test]
fn smoke_case_glob_pattern() {
    let (status, out) =
        run_via_zsh_pipeline("case file.txt in *.txt) echo txt ;; *) echo o ;; esac");
    assert_eq!(status, 0);
    assert_eq!(out, "txt\n");
}

#[test]
fn smoke_repeat() {
    let (status, out) = run_via_zsh_pipeline("repeat 3 echo hi");
    assert_eq!(status, 0);
    assert_eq!(out, "hi\nhi\nhi\n");
}

#[test]
fn smoke_break() {
    let (status, out) = run_via_zsh_pipeline(
        "for i in 1 2 3 4 5; do [[ $i -eq 3 ]] && break; echo $i; done",
    );
    assert_eq!(status, 0);
    assert_eq!(out, "1\n2\n");
}

#[test]
fn smoke_continue() {
    let (status, out) = run_via_zsh_pipeline(
        "for i in 1 2 3; do [[ $i -eq 2 ]] && continue; echo $i; done",
    );
    assert_eq!(status, 0);
    assert_eq!(out, "1\n3\n");
}

// ── Conditionals ──

#[test]
fn smoke_cond_string_eq() {
    let (status, out) = run_via_zsh_pipeline("[[ a == a ]] && echo y");
    assert_eq!(status, 0);
    assert_eq!(out, "y\n");
}

#[test]
fn smoke_cond_num_lt() {
    let (status, out) = run_via_zsh_pipeline("[[ 1 -lt 2 ]] && echo y");
    assert_eq!(status, 0);
    assert_eq!(out, "y\n");
}

#[test]
fn smoke_cond_file_dir() {
    let (status, out) = run_via_zsh_pipeline("[[ -d /tmp ]] && echo y");
    assert_eq!(status, 0);
    assert_eq!(out, "y\n");
}

#[test]
fn smoke_cond_negate() {
    let (status, out) = run_via_zsh_pipeline("[[ ! a == b ]] && echo y");
    assert_eq!(status, 0);
    assert_eq!(out, "y\n");
}

// ── Arithmetic ──

#[test]
fn smoke_arith_compound() {
    let (status, out) = run_via_zsh_pipeline("(( x = 2 + 3 )); echo $x");
    assert_eq!(status, 0);
    assert_eq!(out, "5\n");
}

#[test]
fn smoke_arith_compare() {
    let (status, _) = run_via_zsh_pipeline("(( 1 < 2 ))");
    assert_eq!(status, 0);
    let (status, _) = run_via_zsh_pipeline("(( 0 ))");
    assert_eq!(status, 1);
}
