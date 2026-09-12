//! `zsh/zpty` against a child that has already exited.
//!
//! `checkptycmd` (Src/Modules/zpty.c:533) decides whether a pty
//! command is still running by calling `kill(cmd->pid, 0)`. That test
//! is only accurate once the child has been REAPED — kill(2) succeeds
//! on an exited-but-unreaped zombie. zsh reaps it in the process-wide
//! SIGCHLD handler (`wait_for_processes`, Src/signals.c:285, which
//! loops on `waitpid(-1, …, WNOHANG)` and therefore collects children
//! that are in no job table, as a zpty child deliberately is —
//! zpty.c:348 `clearjobtab(0)`).
//!
//! zshrs reaped that pid nowhere, so `kill(pid, 0)` kept succeeding and
//! `cmd->fin` was never set. `ptywritestr`'s failing-write retry
//! (zpty.c:730-735: `checkptycmd`; if not finished, `written = 0` and
//! go round again) then had no exit condition: `zpty -w` to a child
//! that had already exited spun at 100% CPU forever instead of
//! returning 2.
//!
//! Every case here runs under a hard timeout: a regression must REPORT
//! rather than wedge the suite, because `zsh/zpty` is what this
//! project's own interactive parity harness (`zpty_probe.rs` and
//! everything built on it) uses to drive a shell.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Wall-clock ceiling for one shell invocation. Every script below
/// finishes in ~1-2s when the shells behave; anything near this bound
/// is the spin, not slowness.
const TIMEOUT: Duration = Duration::from_secs(30);

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
    if std::path::Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if std::path::Path::new("/usr/local/bin/zsh").exists() {
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

/// Run one shell with a deadline. `None` means the shell was still
/// alive when the deadline passed and has been killed — i.e. the bug.
///
/// Deliberately hand-rolled instead of `Command::output()`: that call
/// blocks until the child exits, which is exactly what a spinning
/// `zpty -w` never does. The child is killed on timeout so a failing
/// run leaves no 100%-CPU process behind.
fn run_bounded(mut cmd: Command) -> Option<(String, i32)> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shell");

    let deadline = Instant::now() + TIMEOUT;
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                let mut out = String::new();
                if let Some(mut s) = child.stdout.take() {
                    let _ = s.read_to_string(&mut out);
                }
                return Some((out, status.code().unwrap_or(-1)));
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn run_zsh(script: &str) -> Option<(String, i32)> {
    let mut c = Command::new(zsh_path());
    c.args(["-fc", script]);
    run_bounded(c)
}

fn run_zshrs(script: &str) -> Option<(String, i32)> {
    let mut c = Command::new(zshrs_bin());
    c.args(["--zsh", "-f", "-c", script]).env_remove("ZSHRS_CACHE");
    run_bounded(c)
}

/// The listing prints the live child's pid (`(1234) w: cat`), which
/// differs between the two shells by construction. Collapse it so the
/// rest of the line still gets compared.
fn mask_pid(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let (head, tail) = rest.split_at(open);
        out.push_str(head);
        match tail.find(')') {
            Some(close) if tail[1..close].chars().all(|c| c.is_ascii_digit()) && close > 1 => {
                out.push_str("(PID)");
                rest = &tail[close + 1..];
            }
            _ => {
                out.push('(');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Drive one pty command through the whole surface: the listing,
/// `-t`, `-w`, `-t` again, `-r`, `-d`, and the listing afterwards.
/// `spawn` is the `zpty -b w …` argument text.
///
/// The read polls `-r -t` instead of taking one shot at it. `zpty -b`
/// puts the master fd in non-blocking mode, so even a plain `-r` gives
/// up with 1 ("nothing yet") when the write hasn't made the round trip
/// through the pty — a pure race against the child, and the two shells
/// lost it on different runs. Polling makes both sides settle on the
/// answer that is about the command rather than about the timing: 0
/// once the live child echoes, 2 immediately for a finished one (the
/// `if (p->fin) return 2` at zpty.c:802 runs before any read).
fn matrix_script(spawn: &str) -> String {
    format!(
        r#"zmodload zsh/zpty
unset zptyout
zpty -b w {spawn}
print "b=$?"
sleep 1
zpty
print "list=$?"
zpty -t w
print "t=$?"
zpty -w w hello
print "w=$?"
zpty -t w
print "t2=$?"
zptyrc=1
for i in {{1..60}}; do
  zpty -r -t w zptyout
  zptyrc=$?
  [[ $zptyrc -ne 1 ]] && break
  sleep 0.25
done
print "r=$zptyrc out=${{zptyout:-<empty>}}"
zpty -d w
print "d=$?"
zpty
print "list2=$?"
"#
    )
}

fn assert_matrix_parity(case: &str, spawn: &str) {
    if !zsh_available() {
        eprintln!("skip: no zsh at {}", zsh_path());
        return;
    }
    let script = matrix_script(spawn);

    let z = run_zsh(&script).unwrap_or_else(|| {
        panic!("[{case}] the ORACLE {} hung past {TIMEOUT:?}", zsh_path())
    });
    let r = run_zshrs(&script).unwrap_or_else(|| {
        panic!(
            "[{case}] zshrs hung past {TIMEOUT:?} on:\n{script}\n\
             A pty command whose child has exited must be reported \
             finished, not written to forever (zpty.c:533 checkptycmd)."
        )
    });

    assert_eq!(
        mask_pid(&z.0),
        mask_pid(&r.0),
        "[{case}] stdout divergence on:\n{script}"
    );
    assert_eq!(z.1, r.1, "[{case}] exit divergence on:\n{script}");
}

/// Child exited 0 before the write. Every step must report the
/// command finished: listing `(finished)`, `-t` 1, `-w` 2, `-r` 2.
#[test]
fn zpty_write_to_child_that_exited_cleanly() {
    assert_matrix_parity("clean", "true");
}

/// Same, for a child that exited non-zero — zpty exposes no exit
/// status, so this must be indistinguishable from the clean exit.
#[test]
fn zpty_write_to_child_that_exited_nonzero() {
    assert_matrix_parity("nonzero", "false");
}

/// Same, for a child killed by a signal.
#[test]
fn zpty_write_to_child_killed_by_signal() {
    assert_matrix_parity("signal", r#""sh -c 'kill -9 \$\$'""#);
}

/// The control: a child that is still running must NOT be reported
/// finished, must accept the write, and must echo it back. Guards the
/// fix against the opposite error — reaping or declaring death too
/// eagerly, which would break the interactive harness.
#[test]
fn zpty_write_to_running_child() {
    assert_matrix_parity("running", "cat");
}

/// End-to-end shape the parity harness itself depends on: drive an
/// interactive zshrs over a pty and get an answer back. Regression
/// guard for 5929e82b81, where the shell on the far end of the pty
/// never emitted a single byte — which scored `zle_editor_params`,
/// `zle_bufstack` and `prompt_render` at 0 passed / 27 failed.
///
/// The assertion is "it answered", not "it echoed this exact string":
/// ZLE interleaves colour escapes and autosuggestion redraws into the
/// echo, so the written text comes back split across escape sequences
/// and no literal substring survives reliably.
#[test]
fn zpty_interactive_shell_still_answers() {
    let bin = zshrs_bin();
    let script = format!(
        r#"zmodload zsh/zpty
buf=
zpty -b w {} -f -i
print "b=$?"
# Wait for the shell to come up rather than guessing a sleep: a debug
# build under a loaded test runner takes its time.
for i in {{1..60}}; do
  if zpty -r -t w chunk; then buf="$buf$chunk"; fi
  [[ -n $buf ]] && break
  sleep 0.25
done
zpty -w w 'print PTYMARKER'
print "w=$?"
for i in {{1..60}}; do
  if zpty -r -t w chunk; then buf="$buf$chunk"; fi
  [[ ${{#buf}} -gt 40 ]] && break
  sleep 0.25
done
[[ -n $buf ]] && print ANSWERED || print SILENT
zpty -d w
"#,
        bin.display()
    );
    let r = run_zshrs(&script)
        .unwrap_or_else(|| panic!("zshrs hung past {TIMEOUT:?} driving an interactive zshrs"));
    assert!(
        r.0.contains("b=0"),
        "zpty could not start an interactive zshrs: {:?}",
        r.0
    );
    assert!(
        r.0.contains("w=0"),
        "zpty -w to a live interactive shell did not succeed: {:?}",
        r.0
    );
    assert!(
        r.0.contains("ANSWERED"),
        "the interactive shell on the far end of the pty never emitted a byte: {:?}",
        r.0
    );
}
