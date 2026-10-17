//! The interactive prompt must come back after every command.
//!
//! Regression for a hang on Linux: `zhandler` (the SIGCHLD handler) queued a
//! signal by writing `signal_mask_queue`, a lazily built table whose first use
//! allocates. When the child of the first external command exited while the main
//! thread was inside `malloc` (or any code holding the allocator lock), the
//! handler allocated too and the shell waited on itself for ever: output of
//! `ls`, no prompt, `^C` echoed. About one start in five hit it.
//!
//! The harness runs real interactive sessions on a pty with a fixed `PS1`, sends
//! a command and requires the prompt to be painted again. Two shapes matter and
//! both are covered: many FRESH sessions each running their first command (the
//! lazy table is built at the first queued signal), and one long session that runs
//! every command in `COMMANDS` back to back (the table already exists; this
//! guards the rest of the handler path).

#![cfg(unix)]

use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PROMPT: &str = "PROMPT_READY> ";

fn zshrs_bin() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/zshrs")
}

struct Session {
    /// Closed FIRST on drop: a session leader cannot finish exiting while its
    /// terminal's output is still queued for a master nobody reads (macOS leaves the
    /// process in the exiting state for ever, and `wait` with it).
    master: Option<std::fs::File>,
    child: std::process::Child,
    seen: Vec<u8>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        drop(self.master.take());
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Session {
    /// `fx` keeps the native editor engines (syntax highlighting, autosuggest) on.
    /// They repaint on every key, which is slow in a debug build under load; the
    /// signal path under test does not depend on them, so most cases run without.
    fn start(home: &Path, fx: bool) -> Session {
        std::fs::create_dir_all(home).unwrap();
        std::fs::write(home.join(".zshrc"), format!("PS1='{PROMPT}'\nRPS1=\n")).unwrap();
        // The commands are written to the pty in one burst, not typed. Autopair
        // answers a burst `{` / `(` / `"` with a closer that the burst's own closer
        // then fails to overtype, which leaves the line unbalanced: that is the
        // paste case, unrelated to the signal and fork paths under test.
        std::fs::create_dir_all(home.join(".zshrs")).unwrap();
        std::fs::write(home.join(".zshrs/zshrs.toml"), "[zle]\nautopair = false\n").unwrap();
        let mut master: libc::c_int = 0;
        let mut slave: libc::c_int = 0;
        let mut ws = libc::winsize { ws_row: 40, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
        let rc = unsafe {
            libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws)
        };
        assert_eq!(rc, 0, "openpty");
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        let mut cmd = Command::new(zshrs_bin());
        cmd.env_clear()
            .env("HOME", home)
            .env("ZSHRS_HOME", home.join(".zshrs"))
            .env("TERM", "xterm-256color")
            .env("ZSHRS_NATIVE_ZLE_FX", if fx { "1" } else { "0" })
            .env("PATH", "/usr/bin:/bin")
            .current_dir(home)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let child = cmd.spawn().expect("spawn zshrs");
        let master = unsafe { std::fs::File::from_raw_fd(master) };
        Session { master: Some(master), child, seen: Vec::new() }
    }

    fn prompts(&self) -> usize {
        String::from_utf8_lossy(&self.seen).matches(PROMPT).count()
    }

    /// Pump the pty for up to `max`, until `done(&seen)` holds. Answers the
    /// terminal queries the line editor sends so it never waits on them.
    fn pump(&mut self, max: Duration, done: impl Fn(&[u8]) -> bool) -> bool {
        let start = Instant::now();
        let fd = self.master.as_ref().unwrap().as_raw_fd();
        while start.elapsed() < max {
            if done(&self.seen) {
                return true;
            }
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            if unsafe { libc::poll(&mut pfd, 1, 50) } <= 0 {
                continue;
            }
            let mut buf = [0u8; 8192];
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            let chunk = &buf[..n as usize];
            self.seen.extend_from_slice(chunk);
            let s = String::from_utf8_lossy(chunk);
            for _ in 0..s.matches("\x1b[6n").count() {
                let _ = self.master.as_mut().unwrap().write_all(b"\x1b[1;1R");
            }
            if s.contains("\x1b[c") {
                let _ = self.master.as_mut().unwrap().write_all(b"\x1b[?62;c");
            }
        }
        done(&self.seen)
    }

    /// Wait for the first prompt.
    fn wait_for_first_prompt(&mut self, max: Duration) -> bool {
        self.pump(max, |b| has(b, PROMPT.as_bytes()))
    }

    /// Run `cmd` and require the prompt to be painted AGAIN after the command's
    /// own output. The command is followed by `echo D$((6*7))_N`: the typed text
    /// holds `$((6*7))` while the output holds `42`, so only the real output
    /// matches. The line editor repaints the prompt while a line is typed, so a
    /// prompt count alone would be satisfied before the command even ran.
    fn run(&mut self, cmd: &str, n: usize, max: Duration) -> Result<(), String> {
        let marker = format!("D42_{n}\r\n");
        self.master
            .as_mut()
            .unwrap()
            .write_all(format!("{cmd}; echo D$((6*7))_{n}\r").as_bytes())
            .unwrap();
        let ok = self.pump(max, |b| match find(b, marker.as_bytes()) {
            Some(i) => has(&b[i + marker.len()..], PROMPT.as_bytes()),
            None => false,
        });
        if ok {
            Ok(())
        } else {
            let seen = String::from_utf8_lossy(&self.seen);
            let ran = seen.contains(&marker);
            Err(format!(
                "after `{cmd}`: command {}; …{:?}",
                if ran { "ran but no prompt followed" } else { "never finished" },
                self.tail()
            ))
        }
    }

    fn tail(&self) -> String {
        let s = String::from_utf8_lossy(&self.seen);
        let n = s.chars().count();
        s.chars().skip(n.saturating_sub(300)).collect()
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn has(hay: &[u8], needle: &[u8]) -> bool {
    find(hay, needle).is_some()
}

fn home(tag: &str, n: usize) -> PathBuf {
    std::env::temp_dir().join(format!("zshrs-prompt-{tag}-{}-{n}", std::process::id()))
}

/// External commands, pipelines, substitutions, jobs and subshells: each ends in
/// at least one SIGCHLD, delivered at an arbitrary point of the main thread.
const COMMANDS: &[&str] = &[
    "ls",
    "/bin/ls /",
    "true",
    "false",
    "/usr/bin/env true",
    "sh -c 'exit 3'",
    "echo hi | cat",
    "ls | wc -l",
    "cat /dev/null",
    "(echo sub)",
    "echo $(echo cs)",
    "echo $(ls | wc -l)",
    "x=$(/bin/echo captured); echo $x",
    "sleep 0.01 &",
    "sleep 0.01 & wait",
    "for i in 1 2 3; do /usr/bin/true; done",
    "for i in 1 2 3 4 5 6 7 8; do /usr/bin/true & done; wait",
    "a=(); for i in {1..300}; do a+=($i); /usr/bin/true; done",
    "a=(); for i in {1..300}; do a+=($i); /usr/bin/true & done; wait",
    "/usr/bin/true; /usr/bin/true; /usr/bin/true; /usr/bin/true",
    "ls >/dev/null 2>&1",
    "{ /usr/bin/true; } 2>/dev/null",
    "if /usr/bin/true; then /usr/bin/true; fi",
    "while /usr/bin/false; do :; done",
];

/// Fresh sessions: the first queued SIGCHLD of the process, which is when the
/// lazily built signal tables used to be allocated inside the handler.
#[test]
fn fresh_sessions_get_a_prompt_after_their_first_command() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let mut failures = Vec::new();
    for n in 0..24 {
        let h = home("fresh", n);
        let _ = std::fs::remove_dir_all(&h);
        let mut s = Session::start(&h, false);
        assert!(s.wait_for_first_prompt(Duration::from_secs(30)), "no first prompt: {}", s.tail());
        // A short pause lets the startup threads reach their steady state, the
        // way a person typing the first command does.
        std::thread::sleep(Duration::from_millis(20 * (n as u64 % 5)));
        let cmd = COMMANDS[n % COMMANDS.len()];
        if let Err(e) = s.run(cmd, 1, Duration::from_secs(20)) {
            failures.push(format!("session {n} {e}"));
        }
        drop(s);
        let _ = std::fs::remove_dir_all(&h);
    }
    assert!(failures.is_empty(), "prompt never returned:\n{}", failures.join("\n"));
}

/// One long session: every command in turn, then the whole list again.
#[test]
fn one_session_gets_a_prompt_after_every_command_twice() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let h = home("long", 0);
    let _ = std::fs::remove_dir_all(&h);
    let mut s = Session::start(&h, false);
    assert!(s.wait_for_first_prompt(Duration::from_secs(30)), "no first prompt: {}", s.tail());
    let mut n = 0;
    for round in 0..2 {
        for cmd in COMMANDS {
            n += 1;
            if let Err(e) = s.run(cmd, n, Duration::from_secs(20)) {
                panic!("round {round} {e}");
            }
        }
    }
    drop(s);
    let _ = std::fs::remove_dir_all(&h);
}

/// A long foreground command must be interruptible: the prompt returns after ^C.
#[test]
fn prompt_returns_after_interrupting_a_foreground_command() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let h = home("intr", 0);
    let _ = std::fs::remove_dir_all(&h);
    let mut s = Session::start(&h, false);
    assert!(s.wait_for_first_prompt(Duration::from_secs(30)), "no first prompt: {}", s.tail());
    for i in 0..5 {
        s.master.as_mut().unwrap().write_all(b"sleep 30\r").unwrap();
        // Interrupt only once the command has started (`OSC 133;C`). A ^C that
        // lands while the shell is still between the editor and the fork goes to
        // the shell, not to `sleep`.
        assert!(
            s.pump(Duration::from_secs(10), |b| b.windows(7).filter(|w| *w == b"\x1b]133;C").count() > i),
            "`sleep 30` never started: …{:?}",
            s.tail()
        );
        std::thread::sleep(Duration::from_millis(300));
        let before = s.seen.len();
        s.master.as_mut().unwrap().write_all(b"\x03").unwrap();
        // The prompt must be painted after the interrupt.
        let ok = s.pump(Duration::from_secs(10), |b| before <= b.len() && has(&b[before..], PROMPT.as_bytes()));
        assert!(ok, "no prompt after ^C on `sleep 30` (round {i}): …{:?}", s.tail());
    }
    drop(s);
    let _ = std::fs::remove_dir_all(&h);
}

/// The same first-command race with the editor engines on (the configuration a
/// real session runs): their worker threads allocate concurrently with the
/// main thread, which is what made the allocator-lock window wide.
#[test]
fn fresh_sessions_with_editor_engines_get_a_prompt_after_their_first_command() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let mut failures = Vec::new();
    for n in 0..8 {
        let h = home("fx", n);
        let _ = std::fs::remove_dir_all(&h);
        let mut s = Session::start(&h, true);
        assert!(s.wait_for_first_prompt(Duration::from_secs(60)), "no first prompt: {}", s.tail());
        let cmd = ["ls", "true", "echo hi | cat", "sleep 0.01 & wait"][n % 4];
        if let Err(e) = s.run(cmd, 1, Duration::from_secs(90)) {
            failures.push(format!("session {n} {e}"));
        }
        drop(s);
        let _ = std::fs::remove_dir_all(&h);
    }
    assert!(failures.is_empty(), "prompt never returned:\n{}", failures.join("\n"));
}

/// A forked pipeline stage must be able to exit. `echo hi | cat` forks the `echo`
/// stage without exec; it used to leave through `std::process::exit`, whose
/// thread-local destructors wait on threads a fork does not copy. The stage never
/// exited, its end of the pipe stayed open and `cat` never saw EOF (about one run
/// in ten with the editor engines on). Every shape of forked stage is run.
#[test]
fn forked_pipeline_stages_exit_and_release_the_pipe() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    const STAGES: &[&str] = &[
        "echo hi | cat",
        "echo hi | cat | cat",
        "{ echo a; echo b; } | cat",
        "(echo sub) | cat",
        "echo $(echo cs) | cat",
        "print -r -- x | cat",
        "echo hi | /usr/bin/cat",
        "/usr/bin/true | cat",
        "echo hi |& cat",
        "echo hi | cat; echo hi | cat",
        "echo hi > /dev/null | cat",
        "for i in 1 2 3; do echo $i; done | cat",
    ];
    let mut failures = Vec::new();
    for round in 0..2 {
        for (n, cmd) in STAGES.iter().enumerate() {
            let h = home("stage", round * 100 + n);
            let _ = std::fs::remove_dir_all(&h);
            let mut s = Session::start(&h, true);
            assert!(s.wait_for_first_prompt(Duration::from_secs(60)), "no first prompt: {}", s.tail());
            if let Err(e) = s.run(cmd, 1, Duration::from_secs(60)) {
                failures.push(format!("round {round} {e}"));
            }
            drop(s);
            let _ = std::fs::remove_dir_all(&h);
        }
    }
    assert!(failures.is_empty(), "a pipeline stage never exited:\n{}", failures.join("\n"));
}
