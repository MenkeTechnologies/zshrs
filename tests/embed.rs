//! `zsh::run_program`: each call is a forked `zsh -f -c` child whose fds 0-2
//! are pipes, so these run in the ordinary parallel harness — the capture
//! never touches this process's own fds.
#![cfg(unix)]

use zsh::run_program;

#[test]
fn stdout_is_returned_and_exit_zero_keeps_output_so_far() {
    assert_eq!(run_program("print hello", "").as_deref(), Ok("hello\n"));
    assert_eq!(
        run_program("print a; exit 0; print b", "").as_deref(),
        Ok("a\n")
    );
}

#[test]
fn input_is_the_programs_stdin() {
    let out = run_program(
        r#"read x; print "<$x>"; read y; print "[$y]""#,
        "abc\ndef\n",
    );
    assert_eq!(out.as_deref(), Ok("<abc>\n[def]\n"));
}

/// An external command writes fd 1 itself; an in-process buffer would miss it.
#[test]
fn external_command_output_is_captured() {
    assert_eq!(run_program("/bin/echo ext", "").as_deref(), Ok("ext\n"));
}

#[test]
fn nonzero_status_returns_err_with_stderr() {
    let err = run_program("print -u2 oops; exit 3", "").unwrap_err();
    assert!(err.contains("exit status 3"), "{err:?}");
    assert!(err.contains("oops"), "{err:?}");

    let err = run_program("if true; then", "").unwrap_err();
    assert!(
        err.starts_with("exit status "),
        "parse error must fail: {err:?}"
    );
}

/// zshrs keeps these in process-wide tables with no reset path; within one
/// process a second `ShellExecutor` sees all four. Each call must not.
#[test]
fn no_state_leaks_between_calls() {
    let set = "leakvar=1; setopt extendedglob; alias leakalias=x; leakfn() { :; }; print set";
    assert_eq!(run_program(set, "").as_deref(), Ok("set\n"));
    let probe = r#"print -r -- "${leakvar-unset}" ${options[extendedglob]} ${+aliases[leakalias]} ${+functions[leakfn]}"#;
    assert_eq!(run_program(probe, "").as_deref(), Ok("unset off 0 0\n"));
}

#[test]
fn concurrent_calls_each_get_only_their_own_output() {
    let handles: Vec<_> = (0..8)
        .map(|i| {
            std::thread::spawn(move || {
                let src = format!("print a-{i}; sleep 0.2; /bin/echo b-{i}; read x; print $x");
                (i, run_program(&src, &format!("in-{i}\n")))
            })
        })
        .collect();
    for h in handles {
        let (i, out) = h.join().expect("thread");
        assert_eq!(out, Ok(format!("a-{i}\nb-{i}\nin-{i}\n")));
    }
}

/// The child is a copy of this process, taken while another thread holds a
/// lock. It must finish without ever needing that lock.
#[test]
fn a_lock_held_by_another_host_thread_does_not_block_the_child() {
    use std::sync::{mpsc, Mutex};
    static HOST_LOCK: Mutex<()> = Mutex::new(());
    let (locked_tx, locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let _guard = HOST_LOCK.lock().unwrap();
        locked_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    locked_rx.recv().unwrap();
    let out = run_program("print still-runs", "");
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(out.as_deref(), Ok("still-runs\n"));
}
