//! `f`/`t`/`F`/`T` in vi command mode read their target key with
//! `vigetkey`, which calls `getbyte(0L, NULL, 1)` (c:Src/Zle/zle_vi.c:134):
//! `do_keytmout` is 0, so the read waits for as long as the user takes.
//!
//! The port passed 1, arming $KEYTIMEOUT. A target key typed later than that
//! read as EOF, `f` failed, and the target key then ran as a command of its
//! own: `0 f b x` on `echo abc` turned `b` into vi-backward-word and `x`
//! deleted the `e`, running `cho abc` where zsh runs `echo ac`.
//!
//! The test types the target key well after a 10ms KEYTIMEOUT and checks the
//! command line that is accepted.

#![cfg(unix)]

use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn zshrs_bin() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/zshrs")
}

struct Pty {
    master: std::fs::File,
    child: std::process::Child,
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_in_pty(home: &std::path::Path) -> Pty {
    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;
    let mut ws = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    };
    assert_eq!(rc, 0, "openpty");
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    let mut cmd = Command::new(zshrs_bin());
    cmd.env_clear()
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("TERM", "xterm-256color")
        .env("PATH", "/usr/bin:/bin")
        .current_dir(home)
        .stdin(Stdio::from(slave.try_clone().expect("dup slave")))
        .stdout(Stdio::from(slave.try_clone().expect("dup slave")))
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
    Pty { master, child }
}

/// Read until the stream has been quiet for `quiet` (or `max` elapsed),
/// answering the terminal queries the line editor sends so it never waits.
fn drain(pty: &mut Pty, quiet: Duration, max: Duration) -> Vec<u8> {
    let mut out = Vec::new();
    let start = Instant::now();
    let mut last = Instant::now();
    let fd = pty.master.as_raw_fd();
    while start.elapsed() < max && (out.is_empty() || last.elapsed() < quiet) {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut pfd, 1, 50) } <= 0 {
            continue;
        }
        let mut buf = [0u8; 8192];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        out.extend_from_slice(chunk);
        last = Instant::now();
        let s = String::from_utf8_lossy(chunk);
        for _ in 0..s.matches("\x1b[6n").count() {
            let _ = pty.master.write_all(b"\x1b[1;1R");
        }
        if s.contains("\x1b[c") {
            let _ = pty.master.write_all(b"\x1b[?62;c");
        }
    }
    out
}

/// Drop CSI (`ESC [ ... final`) and OSC (`ESC ] ... ST|BEL`) sequences: the
/// command's output arrives wrapped in OSC 133 shell-integration marks.
fn strip_escapes(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                while let Some(d) = it.next() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(d) = it.next() {
                    if d == '\x07' {
                        break;
                    }
                    if d == '\x1b' {
                        it.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn find_char_target_typed_after_keytimeout_is_still_the_target() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let home = std::env::temp_dir().join(format!("zshrs-vi-find-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".zshrc"), "bindkey -v\nKEYTIMEOUT=1\nPS1='> '\n").unwrap();
    let mut pty = spawn_in_pty(&home);
    let first = drain(&mut pty, Duration::from_millis(1200), Duration::from_secs(15));
    assert!(!first.is_empty(), "no prompt painted");

    let quiet = Duration::from_millis(400);
    let max = Duration::from_secs(6);
    // `f` then its target `b` 400ms later — forty times KEYTIMEOUT.
    for key in [&b"echo abc"[..], b"\x1b", b"0", b"f", b"b", b"x"] {
        pty.master.write_all(key).unwrap();
        drain(&mut pty, quiet, max);
    }
    pty.master.write_all(b"\r").unwrap();
    let out = drain(&mut pty, Duration::from_millis(800), max);
    let _ = std::fs::remove_dir_all(&home);

    let text = strip_escapes(&String::from_utf8_lossy(&out));
    let lines: Vec<&str> = text.split(['\r', '\n']).map(str::trim).collect();
    assert!(
        lines.contains(&"ac"),
        "`echo abc` + `0 f b x` must run `echo ac`; output was {text:?}"
    );
}
