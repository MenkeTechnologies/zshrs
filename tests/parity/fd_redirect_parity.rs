//! File descriptor manipulation parity tests:
//! `exec N<file`, `exec N>file`, `>&N`, `<&N`, `exec N<&-` (close).

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
struct R {
    stdout: String,
    exit: i32,
}
fn run_zsh_in(d: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(d)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs_in(d: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(d)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity_in(d: &Path, s: &str) {
    if !zsh_available() {
        return;
    }
    // Snapshot file contents so zsh + zshrs each start from the same
    // dir state (the second-run sees the first-run's appends
    // otherwise). Same fix as noclobber_parity.
    fn snapshot(d: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(d) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(data) = std::fs::read(&path) {
                        out.push((path, data));
                    }
                }
            }
        }
        out
    }
    fn restore(d: &Path, snap: &[(std::path::PathBuf, Vec<u8>)]) {
        if let Ok(entries) = std::fs::read_dir(d) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && !snap.iter().any(|(p, _)| p == &path) {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        for (path, data) in snap {
            let _ = std::fs::write(path, data);
        }
    }
    let snap = snapshot(d);
    let z = run_zsh_in(d, s);
    restore(d, &snap);
    let r = run_zshrs_in(d, s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}
fn tdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

mod exec_open {
    use super::*;

    /// `exec 3<file` opens file as fd 3.
    #[test]
    fn exec_open_fd_for_read() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "line1\nline2\n").unwrap();
        assert_parity_in(d.path(), "exec 3< in.txt; cat <&3; exec 3<&-");
    }

    /// `exec 3>file` opens for write.
    #[test]
    fn exec_open_fd_for_write() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            "exec 3> out.txt; echo hello >&3; exec 3>&-; cat out.txt",
        );
    }

    /// `exec 3>>file` opens for append.
    #[test]
    fn exec_open_fd_for_append() {
        let d = tdir();
        std::fs::write(d.path().join("out.txt"), "first\n").unwrap();
        assert_parity_in(
            d.path(),
            "exec 3>> out.txt; echo second >&3; exec 3>&-; cat out.txt",
        );
    }
}

mod dup_fd {
    use super::*;

    /// `>&1` duplicates stdout.
    #[test]
    fn dup_stderr_to_stdout() {
        assert_parity_in(Path::new("/tmp"), "echo err 1>&2 2>&1");
    }

    /// `2>&1` redirect stderr to stdout.
    #[test]
    fn redirect_stderr_to_stdout() {
        assert_parity_in(Path::new("/tmp"), "{ echo out; echo err >&2; } 2>&1 | cat");
    }

    /// `&>` shortcut: both stdout+stderr.
    #[test]
    fn ampersand_redirect_both() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            "{ echo out; echo err >&2; } &> combined.txt; cat combined.txt | sort",
        );
    }

    /// `&>>` shortcut: append both.
    #[test]
    fn ampersand_append_both() {
        let d = tdir();
        std::fs::write(d.path().join("c.txt"), "first\n").unwrap();
        assert_parity_in(
            d.path(),
            "{ echo out; echo err >&2; } &>> c.txt; sort c.txt",
        );
    }
}

mod close_fd {
    use super::*;

    /// `exec 3<&-` closes fd 3.
    #[test]
    fn close_fd_after_open() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "data\n").unwrap();
        assert_parity_in(
            d.path(),
            "exec 3< in.txt; exec 3<&-; cat <&3 2>/dev/null; echo exit=$?",
        );
    }

    /// Close stdout via 1>&-.
    #[test]
    fn close_stdout_then_print_errors() {
        assert_parity_in(
            Path::new("/tmp"),
            "{ exec 1>&-; echo hello; } 2>/dev/null; echo done",
        );
    }
}

mod read_from_fd {
    use super::*;

    /// `read -u 3` reads from fd 3.
    #[test]
    fn read_dash_u_from_fd() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "first\nsecond\n").unwrap();
        assert_parity_in(
            d.path(),
            "exec 3< in.txt; read -u 3 line; echo \"[$line]\"; exec 3<&-",
        );
    }

    /// `read -u 3` advances position.
    #[test]
    fn read_dash_u_advances() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "one\ntwo\nthree\n").unwrap();
        assert_parity_in(
            d.path(),
            "exec 3< in.txt; read -u 3 a; read -u 3 b; read -u 3 c; echo \"$a/$b/$c\"; exec 3<&-",
        );
    }
}

mod write_to_fd {
    use super::*;

    /// `print -u 3 msg` writes to fd.
    #[test]
    fn print_dash_u_to_fd() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            "exec 3> out.txt; print -u 3 hello; exec 3>&-; cat out.txt",
        );
    }

    /// `echo msg >&3` writes via redirect.
    #[test]
    fn echo_to_fd_via_redirect() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            "exec 3> out.txt; echo hello >&3; exec 3>&-; cat out.txt",
        );
    }
}

mod multiple_fds {
    use super::*;

    /// Several fds open simultaneously.
    #[test]
    fn three_fds_open_separately() {
        let d = tdir();
        std::fs::write(d.path().join("a.txt"), "data-a\n").unwrap();
        std::fs::write(d.path().join("b.txt"), "data-b\n").unwrap();
        assert_parity_in(d.path(),
            "exec 3< a.txt; exec 4< b.txt; read -u 3 x; read -u 4 y; echo \"$x|$y\"; exec 3<&- 4<&-");
    }
}

mod redirect_in_compound {
    use super::*;

    /// Group redirect.
    #[test]
    fn group_redirect_to_file() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            "{ echo a; echo b; echo c; } > out.txt; cat out.txt",
        );
    }

    /// Subshell redirect.
    #[test]
    fn subshell_redirect() {
        let d = tdir();
        assert_parity_in(d.path(), "(echo a; echo b) > out.txt; cat out.txt");
    }

    /// Function-call redirect.
    #[test]
    fn function_call_with_redirect() {
        let d = tdir();
        assert_parity_in(d.path(), "f() { echo from-f; }; f > out.txt; cat out.txt");
    }
}

mod swap_stdout_stderr {
    use super::*;

    /// Classic swap: 3>&1 1>&2 2>&3.
    #[test]
    fn swap_stdout_and_stderr() {
        assert_parity_in(
            Path::new("/tmp"),
            "{ echo a; echo b >&2; } 3>&1 1>&2 2>&3 3>&- | cat",
        );
    }
}

mod here_doc_to_fd {
    use super::*;

    /// here-doc to fd 3.
    #[test]
    fn heredoc_to_arbitrary_fd() {
        assert_parity_in(
            Path::new("/tmp"),
            r#"cat 0<<EOF
content
EOF
"#,
        );
    }
}
