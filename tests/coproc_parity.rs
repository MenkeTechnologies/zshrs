//! Coprocess parity: `coproc CMD` (par_sublist2 COPROC arm,
//! c:Src/parse.c:864-876) rides the Z_ASYNC job path
//! (c:Src/exec.c:1700-1758) — pipe pair into `coprocin`/`coprocout`
//! (c:Src/exec.c:1708-1727), old-coproc fd closure on replacement
//! (c:Src/exec.c:1710-1712), jobtab registration via spawnjob so
//! `jobs` / `kill %N` see it. Consumers: `print -p`
//! (c:Src/builtin.c:4827), `read -p` (c:Src/builtin.c:6510), and the
//! `<&p` / `>&p` DUP redirect forms (c:Src/exec.c:3894-3912).
//!
//! Bugs #387 / #388 in docs/BUGS.md.
//!
//! All scripts are non-interactive `-fc` and kill or EOF-terminate
//! their own coprocess — no orphan processes survive a test run.
//! PIDs are normalized out of compared output; everything else is
//! byte-compared.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Replace any 3+-digit run with `PID` — process ids differ between
/// the two shells by construction; nothing else in these scripts
/// produces a 3-digit number.
fn normalize_pids(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i - start >= 3 {
                out.push_str("PID");
            } else {
                out.push_str(&s[start..i]);
            }
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

struct R {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: normalize_pids(&String::from_utf8_lossy(&o.stdout)),
        stderr: normalize_pids(&String::from_utf8_lossy(&o.stderr)),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: normalize_pids(&String::from_utf8_lossy(&o.stdout)),
        stderr: normalize_pids(&String::from_utf8_lossy(&o.stderr)),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Byte-parity on pid-normalized stdout + stderr + exit code.
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout mismatch for {:?}\n zsh:   {:?}\n zshrs: {:?}",
        s, z.stdout, r.stdout
    );
    assert_eq!(
        z.stderr, r.stderr,
        "stderr mismatch for {:?}\n zsh:   {:?}\n zshrs: {:?}",
        s, z.stderr, r.stderr
    );
    assert_eq!(
        z.exit, r.exit,
        "exit mismatch for {:?} (zsh {} vs zshrs {})",
        s, z.exit, r.exit
    );
}

// ── #388: print -p / read -p round-trips ───────────────────────────

#[test]
fn coproc_print_p_read_p_roundtrip() {
    // c:Src/builtin.c:4827 (print -p → coprocout) +
    // c:Src/builtin.c:6510 (read -p → coprocin).
    assert_parity("coproc cat; print -p hello; read -p line; print -r $line; kill %1");
}

#[test]
fn coproc_two_writes_two_reads_fifo() {
    // Pipe preserves write order across multiple -p round-trips.
    assert_parity("coproc cat; print -p a; print -p b; read -p x; read -p y; print $x$y; kill %1");
}

// ── #388: jobtab registration (jobs / kill %N) ─────────────────────

#[test]
fn coproc_listed_by_jobs() {
    // c:Src/exec.c:1758 spawnjob — the coproc is a real jobtab entry;
    // printjob shows `[1]  + running    cat` (c:Src/jobs.c:1295).
    assert_parity("coproc cat; jobs; kill %1");
}

#[test]
fn coproc_jobs_l_includes_pid() {
    // c:Src/jobs.c:1281 — `jobs -l` prints the pid column.
    assert_parity("coproc sleep 5; jobs -l; kill %1");
}

#[test]
fn kill_percent_resolves_coproc_job() {
    // getjob `%1` (c:Src/jobs.c:2063) resolves the coproc entry;
    // kill returns 0 and a later `jobs` shows nothing new on stdout.
    assert_parity("coproc cat; kill %1; print rc=$?");
}

// ── #388: replacement lifecycle (c:Src/exec.c:1710-1712) ───────────

#[test]
fn second_coproc_replaces_first() {
    // New coproc closes the old one's fds; -p consumers talk to the
    // NEW coproc. `jobs` then lists the survivor as job 2.
    assert_parity(
        "coproc cat; print -p one; read -p a; coproc cat; print -p two; read -p b; \
         print $a$b; kill %1 2>/dev/null; kill %2 2>/dev/null; jobs",
    );
}

#[test]
fn coproc_eof_via_replacement_coproc_exit() {
    // The canonical EOF idiom: dup the read side to fd 4, then
    // `coproc exit` closes the shell's write end to the old coproc
    // (c:Src/exec.c:1710-1712) — sort sees EOF, flushes, exits.
    assert_parity(
        "coproc sort; print -p c; print -p a; exec 4<&p; coproc exit; \
         while read -u4 l; do print -r $l; done",
    );
}

// ── #387/#388: -p with no coprocess ────────────────────────────────

#[test]
fn print_p_no_coproc_error() {
    // c:Src/builtin.c:4827 — `-p: no coprocess`, rc=1.
    assert_parity("print -p hi");
}

#[test]
fn read_p_no_coproc_error() {
    // c:Src/builtin.c:6510-6515 — fires BEFORE identifier validation,
    // so the bash-habit `read -p prompt var` gets the coproc error.
    assert_parity(r#"echo y | read -p "prompt> " ans"#);
}

#[test]
fn read_up_alias_no_coproc_error() {
    // c:Src/builtin.c:6494 — `-u p` routes to the same coproc gate.
    assert_parity("read -up ans");
}
