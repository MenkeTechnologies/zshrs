//! Tests for `ShellExecutor::execute_script_captured`, the entry point for an
//! embedder that owns the terminal and cannot let shell output reach the real
//! fds.
//!
//! Unlike a single-runtime language, a shell's output is not only its own
//! writes — a forked child inherits fd 1 — so these pin that the capture covers
//! builtins, external commands, and stderr alike.
//!
//! Every test here runs in a child process of its own, via [`isolated`]. The
//! capture points the PROCESS's fd 1 at a temp file, so under the default
//! parallel harness the reporter line libtest prints when a sibling test
//! finishes — `test some_other_name ... ok` — is written into whichever capture
//! happens to be open and comes back as part of the captured string. That is
//! not a bug the shell can fix (see the concurrency contract on
//! `execute_script_captured`); it is the contract, and the tests have to honor
//! it rather than assert around it.

use std::io::Write;
use zsh::ShellExecutor;

/// Set in the child; its presence means "you are already the isolated run".
const ISOLATED_ENV: &str = "ZSHRS_EMBED_CAPTURE_ISOLATED";

/// Re-runs the calling test alone in a child process and returns `false`, so
/// the caller returns without touching fd 1 in the shared harness process.
/// Returns `true` when already inside that child, i.e. "go ahead and run".
///
/// `--test-threads=1 --exact NAME` means exactly one test exists to report on,
/// and libtest emits its `test NAME ...` prefix before the body starts and the
/// ` ok` after it ends — never during the window when fd 1 is the capture.
#[must_use]
fn isolated(test_name: &str) -> bool {
    if std::env::var_os(ISOLATED_ENV).is_some() {
        return true;
    }
    let exe = std::env::current_exe().expect("current_exe");
    let child = std::process::Command::new(exe)
        .args(["--exact", test_name, "--test-threads=1", "--nocapture"])
        .env(ISOLATED_ENV, "1")
        .output()
        .expect("spawn isolated child");
    assert!(
        child.status.success(),
        "isolated run of {test_name} failed ({}):\n--- stdout ---\n{}\n--- stderr ---\n{}",
        child.status,
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr),
    );
    false
}

#[test]
fn captured_runs_cover_builtins_children_stderr_and_state() {
    if !isolated("captured_runs_cover_builtins_children_stderr_and_state") {
        return;
    }
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

/// Two threads capturing at once must not restore each other's fds mid-run:
/// fd 1 belongs to the process, so the capture serializes. Without the lock one
/// of these reads back an empty string.
#[test]
fn concurrent_captures_do_not_clobber_each_other() {
    if !isolated("concurrent_captures_do_not_clobber_each_other") {
        return;
    }
    let handles: Vec<_> = (0..4)
        .map(|i| {
            std::thread::spawn(move || {
                let mut sh = ShellExecutor::new();
                let (_, out) = sh.execute_script_captured(&format!("echo thread-{i}"));
                (i, out)
            })
        })
        .collect();
    for h in handles {
        let (i, out) = h.join().expect("thread");
        assert_eq!(out, format!("thread-{i}"));
    }
}

/// The other half of that lock's story, and the reason every test here forks:
/// serializing captures against each other is all a lock can do. A thread that
/// never calls `execute_script_captured` is not holding it, so its fd-1 writes
/// go into whatever capture is open. Pinning it here keeps the limitation a
/// stated contract instead of a rediscovered flake.
///
/// The ordering is a handshake, not a sleep: the script announces the window is
/// open, the writer writes and then opens the gate, and only then does the
/// script finish. The write is inside the window by construction.
#[test]
fn a_foreign_threads_fd1_write_lands_inside_the_capture() {
    if !isolated("a_foreign_threads_fd1_write_lands_inside_the_capture") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("zshrs-capture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let open = dir.join("window-open");
    let gate = dir.join("gate");
    let _ = std::fs::remove_file(&open);
    let _ = std::fs::remove_file(&gate);

    let (open_w, gate_w) = (open.clone(), gate.clone());
    let writer = std::thread::spawn(move || {
        for _ in 0..500 {
            if open_w.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Raw fd 1, from a thread that has never heard of the capture. Kept
        // alive past the write with ManuallyDrop: dropping a File built from
        // fd 1 would close the process's stdout.
        let mut fd1 = std::mem::ManuallyDrop::new(unsafe {
            <std::fs::File as std::os::unix::io::FromRawFd>::from_raw_fd(1)
        });
        let wrote = fd1.write_all(b"FOREIGN\n").and_then(|()| fd1.flush());
        let opened = std::fs::write(&gate_w, b"go");
        (wrote.is_ok(), opened.is_ok())
    });

    let mut sh = ShellExecutor::new();
    let script = format!(
        ": > {open}; for i in {{1..500}}; do [[ -e {gate} ]] && break; sleep 0.01; done; echo scripted",
        open = open.display(),
        gate = gate.display(),
    );
    let (_, out) = sh.execute_script_captured(&script);

    let (wrote, opened) = writer.join().expect("writer thread");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        wrote && opened,
        "handshake never completed: {wrote} {opened}"
    );

    assert!(
        out.contains("scripted"),
        "script's own output missing: {out:?}"
    );
    assert!(
        out.contains("FOREIGN"),
        "a foreign fd-1 write during the window belongs to the capture — if this \
         stops holding, execute_script_captured's concurrency contract changed \
         and its docs need to change with it: {out:?}"
    );
}
