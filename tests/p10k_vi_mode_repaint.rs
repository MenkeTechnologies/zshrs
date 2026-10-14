//! Issue #5 — with the native p10k engine and `bindkey -v`, every vi mode
//! switch erased output above a multi-row prompt.
//!
//! A keymap switch repainted twice: `zlesetkeymap` called `zle_resetprompt`,
//! and the engine's redraw hook saw the keymap name in its signature change
//! and called it again. Each `redisplay` moves the cursor up `lprompth - 1`
//! rows (c:Src/Zle/zle_refresh.c:2439) and the second started from the
//! already-raised row, so the prompt climbed (rows - 1) per switch and
//! `\e[J` cleared the lines it climbed over.
//!
//! The assertion is on the byte stream a terminal would receive for one
//! `Esc`: after the repaint the cursor must be back on the row it started
//! on (rows moved down == rows moved up). No terminal emulator is needed.

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
        ws_row: 40,
        ws_col: 120,
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

/// Rows moved down minus rows moved up by the cursor-motion bytes in `bytes`.
/// Only the forms the prompt painter emits are counted: `\n`, `\e[B`/`\e[nB`
/// and `\e[A`/`\e[nA`.
fn net_rows_down(bytes: &[u8]) -> i64 {
    let s = String::from_utf8_lossy(bytes);
    let mut net = s.matches('\n').count() as i64;
    let b = s.as_bytes();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i] == 0x1b && b[i + 1] == b'[' {
            let mut j = i + 2;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j < b.len() && (b[j] == b'A' || b[j] == b'B') {
                let n: i64 = s[i + 2..j].parse().unwrap_or(1);
                net += if b[j] == b'B' { n } else { -n };
            }
            i = j;
        }
        i += 1;
    }
    net
}

fn write_home(dir: &std::path::Path, elements: &str, add_newline: bool) {
    std::fs::create_dir_all(dir.join(".config/zsh")).unwrap();
    std::fs::create_dir_all(dir.join("p10k")).unwrap();
    std::fs::write(dir.join("p10k/powerlevel10k.zsh-theme"), "").unwrap();
    std::fs::write(dir.join(".config/zsh/p10k.zsh"), "").unwrap();
    std::fs::write(
        dir.join(".zshrc"),
        format!(
            "bindkey -v\n\
             source \"$HOME/p10k/powerlevel10k.zsh-theme\"\n\
             typeset -g POWERLEVEL9K_PROMPT_ADD_NEWLINE={add_newline}\n\
             typeset -g POWERLEVEL9K_BACKGROUND=\n\
             typeset -g POWERLEVEL9K_LEFT_PROMPT_ELEMENTS=({elements})\n\
             typeset -g POWERLEVEL9K_RIGHT_PROMPT_ELEMENTS=(time)\n\
             typeset -g POWERLEVEL9K_{{LEFT,RIGHT}}_SEGMENT_SEPARATOR=\n\
             typeset -g POWERLEVEL9K_MULTILINE_{{FIRST,NEWLINE,LAST}}_PROMPT_{{PREFIX,SUFFIX}}=\n"
        ),
    )
    .unwrap();
}

/// Switch insert -> command -> insert and return the net row displacement of
/// the bytes each switch produced.
fn displacement_per_switch(tag: &str, elements: &str, add_newline: bool) -> Vec<i64> {
    let home = std::env::temp_dir().join(format!(
        "zshrs-p10k-vi-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    write_home(&home, elements, add_newline);
    let mut pty = spawn_in_pty(&home);
    let first = drain(&mut pty, Duration::from_millis(1200), Duration::from_secs(15));
    assert!(!first.is_empty(), "no prompt painted");
    pty.master.write_all(b"echo one\r").unwrap();
    drain(&mut pty, Duration::from_millis(800), Duration::from_secs(8));
    let mut per_switch = Vec::new();
    for key in [&b"\x1b"[..], b"i", b"\x1b", b"i"] {
        pty.master.write_all(key).unwrap();
        let bytes = drain(&mut pty, Duration::from_millis(700), Duration::from_secs(6));
        per_switch.push(net_rows_down(&bytes));
    }
    let _ = std::fs::remove_dir_all(&home);
    per_switch
}

/// Three-row prompt: blank row from PROMPT_ADD_NEWLINE + two rows (the issue's
/// configuration).
#[test]
fn mode_switch_returns_the_cursor_to_its_row_three_row_prompt() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let d = displacement_per_switch("three", "dir newline prompt_char", true);
    assert_eq!(d, vec![0, 0, 0, 0], "cursor drifted on a vi mode switch");
}

/// One-row prompt: nothing to climb, must stay put.
#[test]
fn mode_switch_returns_the_cursor_to_its_row_one_row_prompt() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let d = displacement_per_switch("one", "dir prompt_char", false);
    assert_eq!(d, vec![0, 0, 0, 0], "cursor drifted on a vi mode switch");
}

/// Two rows (`newline` element, no leading blank).
#[test]
fn mode_switch_returns_the_cursor_to_its_row_two_row_prompt() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs binary not built");
        return;
    }
    let d = displacement_per_switch("two", "dir newline prompt_char", false);
    assert_eq!(d, vec![0, 0, 0, 0], "cursor drifted on a vi mode switch");
}
