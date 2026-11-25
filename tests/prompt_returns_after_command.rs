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
    /// The terminal's XTVERSION answer (`\eP>|name version\e\\`), sent when
    /// the shell asks; none by default, like a terminal that does not know it.
    xtversion: &'static [u8],
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
        Session::start_with(home, fx, &[])
    }

    /// `start`, with extra arguments for the shell (`--fish`).
    fn start_with(home: &Path, fx: bool, args: &[&str]) -> Session {
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
        cmd.args(args);
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
        Session { xtversion: b"", master: Some(master), child, seen: Vec::new() }
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
            if s.contains("\x1b[>0q") {
                let _ = self.master.as_mut().unwrap().write_all(self.xtversion);
            }
            if s.contains("\x1b[c") || s.contains("\x1b[0c") {
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

/// A non-interactive shell with `monitor` set must not stop its own children.
/// `entersubsh` used to call `tcsetpgrp(2, pgrp)` in every child it put in a new
/// process group instead of `attachtty`, which acts only when `jobbing &&
/// interact` (Src/utils.c:4781). On a terminal the child, now in a background
/// group, took SIGTTOU and stopped before exec, and the shell waited on it for
/// ever: the recorder hung on zpwr's banner (`clear`, `eza`) this way.
#[test]
fn monitor_in_a_non_interactive_shell_does_not_stop_children_on_a_tty() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let (mut master, mut slave): (libc::c_int, libc::c_int) = (0, 0);
    let rc = unsafe {
        libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
    };
    assert_eq!(rc, 0, "openpty");
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    let mut cmd = Command::new(zshrs_bin());
    cmd.args(["-fc", "setopt monitor; /usr/bin/true; echo D$((6*7))"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
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
    let mut s = Session { xtversion: b"", master: Some(unsafe { std::fs::File::from_raw_fd(master) }), child, seen: Vec::new() };
    assert!(s.pump(Duration::from_secs(20), |b| has(b, b"D42")), "`/usr/bin/true` never finished: …{:?}", s.tail());
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

/// zleread paints the prompt (Src/Zle/zle_main.c:1353 `zrefresh()`) BEFORE
/// `start_edit()` (c:1362) writes `$zle_bracketed_paste[1]`, and PROMPT_CR's
/// `\r` (c:1321-1322) precedes that paint. zsh 5.9.2 sends
/// `\r\e[0m…\e[J<prompt>\e[K\e[?2004h`; zshrs used to send `\e[?2004h`
/// first and drop the `\r`.
#[test]
fn prompt_is_painted_before_bracketed_paste_is_enabled() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let h = home("bpaste", 0);
    let _ = std::fs::remove_dir_all(&h);
    let mut s = Session::start(&h, false);
    let enable = b"\x1b[?2004h";
    let seen_both = s.pump(Duration::from_secs(30), |b| match find(b, PROMPT.as_bytes()) {
        Some(i) => has(&b[i..], enable),
        None => false,
    });
    assert!(seen_both, "no prompt + bracketed paste: {:?}", s.tail());
    let p = find(&s.seen, PROMPT.as_bytes()).unwrap();
    let e = find(&s.seen, enable).unwrap();
    assert!(p < e, "\\e[?2004h written before the prompt: {:?}", s.tail());
    assert!(
        s.seen[..p].contains(&b'\r'),
        "PROMPT_CR `\\r` missing before the first prompt: {:?}",
        s.tail()
    );
    drop(s);
    let _ = std::fs::remove_dir_all(&h);
}

/// Shell integration (OSC 133, read by zellij, kitty, WezTerm, iTerm2): before
/// each prompt the `A` mark (Src/Zle/zle_main.c:1350-1351), `P;k=i` in front of
/// the prompt text (c:1274), `B` when editing starts (termquery.c:668), `C`
/// before the command runs and `D;<status>` after it. The aid is well formed
/// (`aid=` + 6 base64 chars + the placeholder's last `Z`), not zsh's `pre + 13`
/// copy that overwrites the key, and `D` carries the exit status zsh omits.
#[test]
fn osc_133_marks_frame_the_prompt_and_each_command() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let h = home("osc133", 0);
    let _ = std::fs::remove_dir_all(&h);
    let mut s = Session::start(&h, false);
    assert!(s.wait_for_first_prompt(Duration::from_secs(30)), "no prompt: {:?}", s.tail());
    // Raw lines: the status `D` reports is the line's last command's.
    for (line, done) in [("(exit 3)\r", "\x1b]133;D;3"), ("true\r", "\x1b]133;D;0")] {
        s.master.as_mut().unwrap().write_all(line.as_bytes()).unwrap();
        let ok = s.pump(Duration::from_secs(30), |b| has(b, done.as_bytes()));
        assert!(ok, "no {done:?} after {line:?}: {:?}", s.tail());
    }
    // the third prompt: editing started (`B`) once per prompt
    let third_b = |b: &[u8]| String::from_utf8_lossy(b).matches("\x1b]133;B").count() >= 3;
    assert!(s.pump(Duration::from_secs(30), third_b), "no third prompt: {:?}", s.tail());

    // Every OSC 133 mark, in order, with the variable aid reduced to its shape.
    let text = String::from_utf8_lossy(&s.seen).into_owned();
    let marks: Vec<String> = text
        .split("\x1b]133;")
        .skip(1)
        .map(|m| {
            let m = &m[..m.find("\x1b\\").unwrap_or(m.len())];
            match m.strip_prefix("A;cl=m;aid=") {
                Some(id) if id.len() == 7 && id.ends_with('Z') => "A;cl=m;aid=<id>".to_string(),
                _ => m.to_string(),
            }
        })
        .collect();
    let prompt = ["A;cl=m;aid=<id>", "P;k=i", "B"];
    let expected: Vec<&str> = prompt
        .iter()
        .chain(&["C", "D;3"])
        .chain(&prompt)
        .chain(&["C", "D;0"])
        .chain(&prompt)
        .copied()
        .collect();
    assert_eq!(&marks[..expected.len()], &expected[..], "all marks: {marks:?}");
    drop(s);
    let _ = std::fs::remove_dir_all(&h);
}

/// `--fish` writes fish 4.9.3's marks and titles (terminal.rs:197-226,
/// reader.rs:868-884, 5060-5137, 6398-6407): `A;click_events=1` and `B`
/// around the prompt; for a line, `C;cmdline_url=` with the line AS TYPED (the
/// history holds its zsh translation), then the OSC 0 title with SGR 0 and a
/// carriage return, then `D;<status>`; and a line that exits the shell still
/// gets its `D` with the exit status. The session's PATH has no fish, whose
/// embedded `fish_title` zshrs would load, so the title is fish's default
/// `echo (status current-command) " " $PWD`: the line's first word, three
/// spaces, the directory.
#[test]
fn fish_drop_in_marks_and_titles_each_line_like_fish() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let h = home("fishmarks", 0);
    let _ = std::fs::remove_dir_all(&h);
    let mut s = Session::start_with(&h, false, &["--fish"]);
    let prompt_b = b"\x1b]133;A;click_events=1\x1b\\";
    assert!(s.pump(Duration::from_secs(120), |b| has(b, prompt_b)), "no prompt mark: {:?}", s.tail());
    s.master.as_mut().unwrap().write_all(b"false\r").unwrap();
    assert!(s.pump(Duration::from_secs(30), |b| has(b, b"\x1b]133;D;1\x1b\\")), "no D;1: {:?}", s.tail());
    s.master.as_mut().unwrap().write_all(b"echo $status\r").unwrap();
    assert!(s.pump(Duration::from_secs(30), |b| has(b, b"\x1b]133;D;0\x1b\\")), "no D;0: {:?}", s.tail());
    s.master.as_mut().unwrap().write_all(b"exit 3\r").unwrap();
    assert!(s.pump(Duration::from_secs(30), |b| has(b, b"\x1b]133;D;3\x1b\\")), "no D;3: {:?}", s.tail());

    // C, then the title, then D, for each line
    let seen = s.seen.clone();
    for (line, url, first, status) in [
        ("false", "false", "false", 1),
        ("echo $status", "echo%20%24status", "echo", 0),
        ("exit 3", "exit%203", "exit", 3),
    ] {
        let c = find(&seen, format!("\x1b]133;C;cmdline_url={url}\x1b\\").as_bytes())
            .unwrap_or_else(|| panic!("no C for {line:?}: {:?}", s.tail()));
        let title = format!("\x1b]0;{first}   ");
        let t = c + find(&seen[c..], title.as_bytes()).unwrap_or_else(|| panic!("no title after C for {line:?}"));
        let after_title = &seen[t..];
        let st = find(after_title, b"\x1b\\").unwrap();
        assert!(after_title[st..].starts_with(b"\x1b\\\x1b[m\r"), "title not closed with SGR 0 + CR: {line:?}");
        let d = find(&seen[t..], format!("\x1b]133;D;{status}\x1b\\").as_bytes());
        assert!(d.is_some(), "no D;{status} after the title for {line:?}");
    }
    // the line `echo $status` reported the previous line's status
    assert!(has(&seen, b"\x1b[m\r1\r\n"), "echo $status after false: {:?}", s.tail());
    drop(s);
    let _ = std::fs::remove_dir_all(&h);
}

/// fish writes no OSC 133 marks to a terminal that answers XTVERSION as
/// Konsole (terminal.rs:182-195), but still sets the title.
#[test]
fn fish_drop_in_writes_no_marks_to_konsole() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let h = home("fishkonsole", 0);
    let _ = std::fs::remove_dir_all(&h);
    let mut s = Session::start_with(&h, false, &["--fish"]);
    s.xtversion = b"\x1bP>|Konsole 23.08.5\x1b\\";
    // fish_prompt's `> ` is drawn (cleared to the end of the line) once per prompt
    let prompts = |b: &[u8]| b.windows(5).filter(|w| *w == b"> \x1b[K").count();
    assert!(s.pump(Duration::from_secs(120), |b| prompts(b) >= 1), "no prompt: {:?}", s.tail());
    s.master.as_mut().unwrap().write_all(b"false\r").unwrap();
    let titled = s.pump(Duration::from_secs(30), |b| has(b, b"\x1b]0;false   "));
    assert!(titled, "no command title: {:?}", s.tail());
    assert!(s.pump(Duration::from_secs(30), |b| prompts(b) >= 2), "no second prompt: {:?}", s.tail());
    assert!(!has(&s.seen, b"\x1b]133;"), "OSC 133 written to Konsole: {:?}", s.tail());
    drop(s);
    let _ = std::fs::remove_dir_all(&h);
}
