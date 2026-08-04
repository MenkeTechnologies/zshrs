//! Tests for `ShellExecutor::execute_script_captured`, the entry point for an
//! embedder that owns the terminal and cannot let shell output reach the real
//! fds.
//!
//! Unlike a single-runtime language, a shell's output is not only its own
//! writes — a forked child inherits fd 1 — so these pin that the capture covers
//! builtins, external commands, and stderr alike.
//!
//! All of it lives in ONE test on purpose: the capture swaps the process's fd 1
//! for the duration of a run, which is process-global state. Split across
//! `#[test]` functions, libtest's threads would run two captures at once and
//! each would swallow the other's output.

use zsh::ShellExecutor;

#[test]
fn captured_runs_cover_builtins_children_stderr_and_state() {
    let mut sh = ShellExecutor::new();

    // A builtin's output is captured, and the status comes back with it.
    let (status, out) = sh.execute_script_captured("echo hello");
    assert_eq!((status, out.as_str()), (0, "hello"));

    // A forked child writes fd 1 directly — precisely what an in-process buffer
    // could not catch, and what the pipe does.
    let (status, out) = sh.execute_script_captured("/bin/echo from-a-child");
    assert_eq!((status, out.as_str()), (0, "from-a-child"));

    // stderr is folded into the same capture, so a diagnostic cannot leak onto
    // the embedder's display.
    let (_, out) = sh.execute_script_captured("echo out; echo err >&2");
    assert!(out.contains("out"), "stdout missing: {out:?}");
    assert!(out.contains("err"), "stderr missing: {out:?}");

    // A failing script is distinguishable from one that merely printed nothing.
    let (status, _) = sh.execute_script_captured("false");
    assert_ne!(status, 0);

    // Shell state persists across captured runs, as across ordinary ones — an
    // embedder's REPL keeps its variables.
    let _ = sh.execute_script_captured("typeset -g kept=yes");
    let (_, out) = sh.execute_script_captured("echo $kept");
    assert_eq!(out, "yes");
}
