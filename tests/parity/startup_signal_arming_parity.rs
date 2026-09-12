//! Startup signal-handler arming parity, per ENTRY PATH.
//!
//! `init_signals` (`c:Src/init.c:1427-1470`) is reached by C's `main` on
//! every invocation. zshrs dispatches `-c` and a script FILE inside
//! `bins/zshrs.rs` without going through `ported::init::zsh_main`, so the
//! whole of that setup used to be skipped on both: an untrapped SIGALRM
//! killed `zshrs -f -i -c 'kill -ALRM $$'` with the raw signal where zsh
//! prints `timeout` and exits 14, and SIGHUP/SIGTERM/SIGQUIT/SIGPIPE were
//! wrong on the same paths.
//!
//! Every expectation here is the ORACLE's own answer, read at run time —
//! nothing is hardcoded. Each child is given a deadline so a shell that
//! wedges fails the test instead of the suite.
//!
//! The shell under test signals ITSELF (`kill -SIG $$` inside the child),
//! never the test runner's process group. SIGTSTP is deliberately absent:
//! a stopped child is a wedged child, and `zsh_compat_parity_gaps`
//! already owns that case.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(20);

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

#[derive(Debug, PartialEq, Eq)]
struct Out {
    /// Normal exit status, or `None` when the shell died from a signal.
    code: Option<i32>,
    /// Terminating signal, or `None` on a normal exit. This is the whole
    /// point of the test: an unarmed handler shows up here.
    signal: Option<i32>,
    stdout: String,
    /// stderr with the shell's own name folded to `zsh` so the diagnostic
    /// TEXT ("zsh:1: timeout") is compared, not the argv[0] it carries.
    stderr: String,
}

/// Run one shell to completion under a deadline. A child still alive at
/// the deadline is SIGKILLed and reported as `code: None, signal: None`
/// with a `timed out` marker, which can never equal a live shell's answer.
fn run(bin: &str, args: &[&str], ignore_quit: bool) -> Out {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent-zshrs-parity")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if ignore_quit {
        // The shell must START with SIGQUIT inherited as SIG_IGN — what
        // nohup, a supervisor or a `trap '' QUIT` parent hands it. C only
        // RECORDS that (c:1444-1445) after the interactive branch has
        // already reset every disposition (c:1437-1438), so an
        // interactive shell must NOT report the inherited trap.
        unsafe {
            cmd.pre_exec(|| {
                libc::signal(libc::SIGQUIT, libc::SIG_IGN);
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    let pid = child.id() as libc::pid_t;
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut o = Vec::new();
        let mut e = Vec::new();
        let _ = out.read_to_end(&mut o);
        let _ = err.read_to_end(&mut e);
        let status = child.wait();
        let _ = tx.send((status, o, e));
    });
    let (status, o, e) = match rx.recv_timeout(DEADLINE) {
        Ok(v) => v,
        Err(_) => {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            let _ = rx.recv_timeout(Duration::from_secs(5));
            return Out {
                code: None,
                signal: None,
                stdout: String::new(),
                stderr: format!("<{bin} timed out after {}s>", DEADLINE.as_secs()),
            };
        }
    };
    let status = status.expect("wait");
    use std::os::unix::process::ExitStatusExt;
    let shell_name = Path::new(bin)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Out {
        code: status.code(),
        signal: status.signal(),
        stdout: String::from_utf8_lossy(&o).into_owned(),
        stderr: String::from_utf8_lossy(&e)
            .replace(&format!("{shell_name}:"), "zsh:")
            .replace(&format!("{shell_name} "), "zsh "),
    }
}

/// Compare zshrs against the oracle for one argv, asserting whatever the
/// oracle answers.
fn assert_parity_args(label: &str, args: &[&str], ignore_quit: bool) {
    if !zsh_available() {
        return;
    }
    let want = run(zsh_path(), args, ignore_quit);
    let bin = zshrs_bin();
    let got = run(&bin.to_string_lossy(), args, ignore_quit);
    assert_eq!(
        want, got,
        "{label}: divergence on {args:?}\n--- zsh ---\n{want:#?}\n--- zshrs ---\n{got:#?}"
    );
}

/// `kill -<sig> $$` with no trap, on both `-c` entry paths.
fn assert_untrapped(sig: &str) {
    let code = format!("kill -{sig} $$; print survived");
    assert_parity_args(
        &format!("untrapped {sig} under -i -c"),
        &["-f", "-i", "-c", &code],
        false,
    );
    assert_parity_args(
        &format!("untrapped {sig} under -c"),
        &["-f", "-c", &code],
        false,
    );
}

/// The same signal WITH a trap: this half already worked (the `trap`
/// builtin installs the handler itself), so it pins that the arming fix
/// did not disturb it.
fn assert_trapped(sig: &str) {
    let code = format!("trap 'print caught-{sig}' {sig}; kill -{sig} $$; print survived");
    assert_parity_args(
        &format!("trapped {sig} under -i -c"),
        &["-f", "-i", "-c", &code],
        false,
    );
    assert_parity_args(
        &format!("trapped {sig} under -c"),
        &["-f", "-c", &code],
        false,
    );
}

/// The reported bug: an untrapped alarm. zsh answers `zsh:1: timeout`
/// and exit 14 under `-i`, and death by SIGALRM without it.
#[test]
fn untrapped_alrm_matches_zsh_on_both_c_paths() {
    assert_untrapped("ALRM");
}

/// SIGHUP is the one handler C installs even NON-interactively
/// (c:1451-1454), so both `-c` paths were wrong before the fix.
#[test]
fn untrapped_hup_matches_zsh_on_both_c_paths() {
    assert_untrapped("HUP");
}

/// SIGTERM is ignored outright by an interactive shell (c:1463) and
/// fatal without `-i`.
#[test]
fn untrapped_term_matches_zsh_on_both_c_paths() {
    assert_untrapped("TERM");
}

/// SIGQUIT is ignored on EVERY path (c:1448), interactive or not.
#[test]
fn untrapped_quit_matches_zsh_on_both_c_paths() {
    assert_untrapped("QUIT");
}

/// SIGPIPE gets a handler only when interactive (c:1461); the
/// non-interactive path dies from it.
#[test]
fn untrapped_pipe_matches_zsh_on_both_c_paths() {
    assert_untrapped("PIPE");
}

/// SIGUSR1 already agreed before the fix — pinned so a later change to
/// the arming sequence cannot silently move it.
#[test]
fn untrapped_usr1_matches_zsh_on_both_c_paths() {
    assert_untrapped("USR1");
}

/// OPEN GAP, not a regression: zsh's SIGINT handler does not terminate
/// the shell — it sets `errflag |= ERRFLAG_INT` (c:Src/signals.c:457) and
/// execlist's `while (… && !errflag)` (c:Src/exec.c:1443) ends the list,
/// so zsh exits NORMALLY with 130. zshrs emits no such per-statement gate
/// for a top-level `-c` chunk, so installing the handler would let the
/// rest of the list run (`print survived`, exit 0) — measurably worse
/// than leaving SIGINT at its default disposition, which is what
/// `startup_signals::init_dispatch_signals` does and why `$?` agrees at
/// 130 while the wait status here does not (killed-by-2 vs exited-130).
/// Un-ignore this the day the list gate lands.
#[test]
#[ignore = "no top-level errflag list gate: zshrs dies from SIGINT where zsh exits 130 normally"]
fn untrapped_int_matches_zsh_on_both_c_paths() {
    assert_untrapped("INT");
}

/// SIGWINCH is handled but held behind the standing `winch_block()`
/// (c:1457-1458), so neither shell reacts to it inside a `-c` command.
#[test]
fn winch_matches_zsh_trapped_and_untrapped() {
    assert_untrapped("WINCH");
    assert_trapped("WINCH");
}

#[test]
fn trapped_signals_match_zsh_on_both_c_paths() {
    for sig in ["ALRM", "HUP", "TERM", "QUIT", "PIPE", "INT", "USR1"] {
        assert_trapped(sig);
    }
}

/// The script-FILE dispatch is the second path that skips `init_signals`.
#[test]
fn untrapped_alrm_matches_zsh_on_the_script_file_path() {
    if !zsh_available() {
        return;
    }
    let script = std::env::temp_dir().join(format!(
        "zshrs-parity-alrm-{}-{}.zsh",
        std::process::id(),
        line!()
    ));
    std::fs::write(&script, "kill -ALRM $$\nprint survived\n").expect("write script");
    let path = script.to_string_lossy().into_owned();
    assert_parity_args("untrapped ALRM under -i <script>", &["-f", "-i", &path], false);
    assert_parity_args("untrapped ALRM under <script>", &["-f", &path], false);
    let _ = std::fs::remove_file(&script);
}

/// An INHERITED `SIG_IGN` on SIGQUIT is recorded as an ignored trap only
/// for a shell that never ran C's interactive disposition reset — so
/// `nohup zsh -fic trap` lists nothing while `nohup zsh -fc trap` lists
/// `trap -- '' QUIT`. Doing the record BEFORE the reset, which is what
/// the dispatch paths used to do, gets the interactive half wrong.
#[test]
fn inherited_sigquit_ignore_is_recorded_only_when_not_interactive() {
    assert_parity_args("inherited QUIT ignore, -i -c", &["-f", "-i", "-c", "trap"], true);
    assert_parity_args("inherited QUIT ignore, -c", &["-f", "-c", "trap"], true);
}
