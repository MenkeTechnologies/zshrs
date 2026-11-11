//! A script too large for one bytecode chunk.
//!
//! fusevm addresses a chunk's constant pool with a u16, so one chunk holds at
//! most 65536 constants. A program with more top-level lists than
//! `SEGMENT_LISTS` is compiled as segments, each its own sub-chunk
//! (`compile_zsh::ZshCompiler::compile_segmented`). Generated scripts reach
//! that size: `uv --generate-shell-completion fish` is 3960 `complete` lines.
//! Before the split, compiling such a script panicked ("constant pool is full
//! at 65536 entries"). The expectations are zsh's (checked against zsh 5.9).

use std::process::{Command, Stdio};

fn run(script: &str) -> (String, Option<i32>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("big.zsh");
    std::fs::write(&path, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--zsh", "-f"])
        .arg(&path)
        .env("HOME", dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn zshrs");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code())
}

/// 1500 lists of 60 distinct words each: ~90 000 constants, more than one
/// chunk can address. Every list runs, `$LINENO` keeps counting across the
/// segment boundaries, and a function defined in the first segment shadows
/// the `cat` extension builtin when a later segment calls it.
#[test]
fn a_program_over_the_constant_pool_runs_whole() {
    let mut s = String::from("cat() { print mine $1 }\nn=0\n");
    for i in 0..1500 {
        let words: Vec<String> = (0..60).map(|j| format!("w{i}_{j}")).collect();
        s.push_str(&format!("a=({}); (( n += ${{#a}} ))\n", words.join(" ")));
    }
    s.push_str("print n=$n line=$LINENO\ncat end\n");
    let (out, status) = run(&s);
    assert_eq!(out, "n=90000 line=1503\nmine end\n");
    assert_eq!(status, Some(0));
}

/// `return` from a sourced file, `exit`, and an error each stop the program
/// in whichever segment they occur, as C's list loop stops (c:Src/exec.c:1390).
#[test]
fn return_exit_and_errors_end_a_segmented_program() {
    let body = |stop: &str, at: usize| {
        let mut s = String::new();
        for i in 0..1300 {
            s.push_str(&format!("y={i}\n"));
            if i == at {
                s.push_str(stop);
                s.push('\n');
            }
        }
        s.push_str("print notreached\n");
        s
    };
    assert_eq!(run(&body("exit 3", 900)), (String::new(), Some(3)));
    assert_eq!(run(&body("print ${nope?boom}", 600)), (String::new(), Some(1)));

    let dir = tempfile::tempdir().expect("tempdir");
    let sourced = dir.path().join("ret.zsh");
    std::fs::write(&sourced, body("return 4", 700)).unwrap();
    let (out, _) = run(&format!("source {}; print rc=$?\n", sourced.display()));
    assert_eq!(out, "rc=4\n");
}
