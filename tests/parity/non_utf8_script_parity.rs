//! Parity for script input that is NOT valid UTF-8.
//!
//! C zsh has no UTF-8 requirement on script input: `Src/input.c`
//! reads RAW BYTES and metafies every byte `>= Meta` (`Src/utils.c`
//! `metafy`), so an ISO-8859-1 byte in a comment, a string or a
//! filename survives the lexer untouched and is written back out
//! verbatim. zshrs read every script with `fs::read_to_string`,
//! which rejects the whole file on the first invalid byte — one
//! stray byte in one of the thousands of completion files under
//! `$fpath` made that file unloadable, and on the `source` /
//! `autoload` paths it failed SILENTLY.
//!
//! Each test asserts zshrs's stdout BYTES equal zsh's stdout bytes.
//! The clean-ASCII and valid-multibyte cases are the controls: they
//! are the overwhelmingly common input and must not move.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Run both shells on the same script FILE and return (zsh, zshrs) stdout bytes.
fn run_file(dir: &Path, name: &str, body: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write script");
    let z = Command::new(zsh_path())
        .arg("-f")
        .arg(&path)
        .current_dir(dir)
        .output()
        .expect("zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f"])
        .arg(&path)
        .current_dir(dir)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    (z.stdout, r.stdout)
}

/// Feed the same bytes to both shells on STDIN.
fn run_stdin(dir: &Path, body: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut out = Vec::new();
    for (prog, args) in [
        (zsh_path().to_string(), vec!["-f".to_string()]),
        (
            zshrs_bin().to_string_lossy().into_owned(),
            vec!["--zsh".to_string(), "-f".to_string()],
        ),
    ] {
        let mut child = Command::new(prog)
            .args(&args)
            .current_dir(dir)
            .env_remove("ZSHRS_CACHE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(body)
            .expect("write stdin");
        out.push(child.wait_with_output().expect("wait").stdout);
    }
    (out.remove(0), out.remove(0))
}

/// Run the same `-c` snippet in both shells.
fn run_c(dir: &Path, snippet: &str) -> (Vec<u8>, Vec<u8>) {
    let z = Command::new(zsh_path())
        .args(["-fc", snippet])
        .current_dir(dir)
        .output()
        .expect("zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", snippet])
        .current_dir(dir)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    (z.stdout, r.stdout)
}

fn show(b: &[u8]) -> String {
    b.iter()
        .map(|&c| {
            if (0x20..0x7f).contains(&c) {
                (c as char).to_string()
            } else {
                format!("\\x{:02x}", c)
            }
        })
        .collect()
}

/// `echo caf\xe9` — one ISO-8859-1 byte in the middle of a script.
/// zsh prints `before`, the raw byte, and `after`; zshrs refused the
/// whole file with "stream did not contain valid UTF-8" and ran
/// nothing at all.
#[test]
fn latin1_byte_in_script_file_runs_and_round_trips() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::TempDir::new().expect("tmp");
    let (z, r) = run_file(
        d.path(),
        "latin1.zsh",
        b"echo before\necho caf\xe9\necho after\n",
    );
    assert_eq!(
        show(&z),
        show(&r),
        "script-file arg: zsh vs zshrs stdout bytes"
    );
    assert!(z.windows(4).any(|w| w == b"caf\xe9"), "zsh baseline sanity");
}

/// Same bytes piped on stdin. zshrs replaced the byte with U+FFFD
/// (`ef bf bd`) because the SHIN buffer decoded lossily.
#[test]
fn latin1_byte_on_stdin_round_trips() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::TempDir::new().expect("tmp");
    let (z, r) = run_stdin(d.path(), b"echo before\necho caf\xe9\necho after\n");
    assert_eq!(show(&z), show(&r), "stdin: zsh vs zshrs stdout bytes");
}

/// `source file` — zshrs produced NO output at all (rc 126) rather
/// than an error, which is how this hid inside plugin/completion loads.
#[test]
fn latin1_byte_in_sourced_file_round_trips() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::TempDir::new().expect("tmp");
    std::fs::write(d.path().join("s.zsh"), b"echo before\necho caf\xe9\necho after\n")
        .expect("write");
    let (z, r) = run_c(d.path(), "source ./s.zsh");
    assert_eq!(show(&z), show(&r), "source: zsh vs zshrs stdout bytes");
}

/// The `$fpath` case that motivated this: an autoloaded function
/// whose file carries one legacy byte. zshrs loaded nothing and
/// returned 0 — a silent no-op.
#[test]
fn latin1_byte_in_autoload_file_round_trips() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::TempDir::new().expect("tmp");
    let fns = d.path().join("fns");
    std::fs::create_dir_all(&fns).expect("mkdir");
    std::fs::write(fns.join("myfn"), b"echo fnbefore\necho caf\xe9\n").expect("write");
    let (z, r) = run_c(d.path(), "fpath=(./fns); autoload -Uz myfn; myfn");
    assert_eq!(show(&z), show(&r), "autoload: zsh vs zshrs stdout bytes");
}

/// CONTROL — plain ASCII. The overwhelmingly common script; must be
/// byte-identical before and after the byte-preserving read.
#[test]
fn ascii_script_unchanged() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::TempDir::new().expect("tmp");
    let body = b"x=1\nfor i in a b c; do print -r -- \"$i$x\"; done\nprint -r -- ${#x}\n";
    let (z, r) = run_file(d.path(), "ascii.zsh", body);
    assert_eq!(show(&z), show(&r), "ascii script-file");
    let (z, r) = run_stdin(d.path(), body);
    assert_eq!(show(&z), show(&r), "ascii stdin");
}

/// CONTROL — valid multibyte UTF-8 (CJK + emoji + combining). These
/// are real Unicode characters, NOT raw bytes: they must stay ONE
/// character each (`${#s}`), keep their UTF-8 encoding on output, and
/// must not be metafied into byte pairs.
#[test]
fn multibyte_utf8_script_unchanged() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::TempDir::new().expect("tmp");
    let body = "s=日本語\nprint -r -- $s\nprint -r -- ${#s}\ne=🎉\nprint -r -- \"$e${#e}\"\nprint -r -- é ${#:-é}\n"
        .as_bytes();
    let (z, r) = run_file(d.path(), "utf8.zsh", body);
    assert_eq!(show(&z), show(&r), "utf-8 script-file");
    let (z, r) = run_stdin(d.path(), body);
    assert_eq!(show(&z), show(&r), "utf-8 stdin");
}

/// A latin-1 byte inside a COMMENT — the most common way a legacy
/// byte reaches a completion file — must not stop the code around it.
#[test]
fn latin1_byte_in_comment_does_not_kill_the_file() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::TempDir::new().expect("tmp");
    let (z, r) = run_file(
        d.path(),
        "cmt.zsh",
        b"# written by Jos\xe9\nprint -r -- ok\n",
    );
    assert_eq!(show(&z), show(&r), "latin-1 in comment");
    assert_eq!(show(&r), "ok\\x0a", "the file must still run");
}
