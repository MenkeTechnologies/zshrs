//! An interactive shell installs a SIGPIPE handler (C init_signals,
//! signals.c:1461), so a builtin writing to a closed stdout sees EPIPE.
//! The write must be ignored, not turned into a Rust panic.

use std::process::{Command, Stdio};

/// Run an interactive `-c` script whose stdout reader closes after the
/// first byte; return the shell's stderr.
fn stderr_with_closed_stdout(script: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["-f", "-i", "-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zshrs");
    // Read one byte then drop the pipe: the writer keeps producing output.
    let mut out = child.stdout.take().unwrap();
    let mut first = [0u8; 1];
    std::io::Read::read_exact(&mut out, &mut first).expect("first byte");
    drop(out);
    let result = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&result.stderr).into_owned()
}

#[test]
fn builtin_output_to_closed_pipe_does_not_panic() {
    let err = stderr_with_closed_stdout("print -l {1..300000}");
    assert!(
        !err.contains("panicked") && !err.contains("failed printing"),
        "builtin panicked on EPIPE: {err}"
    );
}

#[test]
fn set_to_closed_pipe_does_not_panic() {
    let err = stderr_with_closed_stdout("set; set; set");
    assert!(
        !err.contains("panicked") && !err.contains("failed printing"),
        "`set` panicked on EPIPE: {err}"
    );
}
