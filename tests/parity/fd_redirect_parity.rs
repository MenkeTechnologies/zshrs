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

    /// `exec N<<<str` must leave fd N OPEN for the rest of the script,
    /// exactly like `exec N<file` does (c:Src/exec.c:3978-3986,
    /// nullexec==1: "we specifically *don't* restore the original
    /// fd's"). It did not: the helper behind it closed the descriptor
    /// it had just installed, so every later `<&N` said "N: bad file
    /// descriptor".
    ///
    /// The cause was a missing arm of `redup` (c:Src/utils.c:2049-2065)
    /// — `zclose(x)` is inside `else if (x != y)`, i.e. a no-op when
    /// the source and target descriptors are the same number. They ARE
    /// the same here: `mkstemp` takes the lowest free fd (3 for
    /// `exec 3<<<…`), closes it at c:4676, and the `open` at c:4677
    /// reclaims that very number.
    #[test]
    fn exec_herestring_to_fd_persists() {
        assert_parity_in(Path::new("/tmp"), "exec 3<<< hello\ncat <&3\n");
    }

    /// Same persistence contract for the here-DOCUMENT spelling.
    #[test]
    fn exec_heredoc_to_fd_persists() {
        assert_parity_in(Path::new("/tmp"), "exec 3<<EOF\nhello\nEOF\ncat <&3\n");
    }

    /// A shell builtin reading the persisted fd, not just an external
    /// command — `read -u3` returned status 1 with an empty line.
    #[test]
    fn exec_heredoc_to_fd_readable_by_read_u() {
        assert_parity_in(
            Path::new("/tmp"),
            "exec 3<<EOF\nhello\nEOF\nread -u3 line\nr=$?\nprint -r -- \"rc=$r [$line]\"\n",
        );
    }

    /// `{varid}<<<str` — c:Src/exec.c:3779 hands `fn->varid` to `addfd`
    /// for REDIR_HERESTR just as it does for REDIR_READ, so the varid
    /// arm (c:2402-2412: `movefd` above 10, FDT_EXTERNAL, `setiparam`)
    /// applies. zshrs ignored the varid on this form, fell through to
    /// the fd-0 pending-stdin path and left $f unset, so `cat <&$f`
    /// died with "file number expected".
    #[test]
    fn exec_named_fd_herestring() {
        assert_parity_in(Path::new("/tmp"), "exec {f}<<< hello\ncat <&$f\n");
    }

    /// `exec {v}<<EOF` already allocated the fd; the body it carried was
    /// the lossy part.
    #[test]
    fn exec_named_fd_heredoc() {
        assert_parity_in(Path::new("/tmp"), "exec {f}<<EOF\nhello\nEOF\ncat <&$f\n");
    }

    /// c:Src/exec.c:4671-4672 — `getherestr` appends the newline only
    /// when `!(fn->flags & REDIRF_FROM_HEREDOC)`. A here-DOCUMENT body
    /// therefore reaches the fd byte-for-byte, trailing blank lines
    /// included. Both fd-bearing arms used to
    /// `trim_end_matches('\n')` and let the helper append one back,
    /// collapsing "hello\n\n\n" to "hello\n".
    #[test]
    fn exec_heredoc_to_fd_keeps_trailing_blank_lines() {
        assert_parity_in(Path::new("/tmp"), "exec 3<<EOF\nhello\n\n\nEOF\ncat <&3\n");
    }

    #[test]
    fn exec_named_fd_heredoc_keeps_trailing_blank_lines() {
        assert_parity_in(
            Path::new("/tmp"),
            "exec {f}<<EOF\nhello\n\n\nEOF\ncat <&$f\n",
        );
    }

    /// The quoted-terminator spelling takes the other branch of both
    /// arms and has the same contract.
    #[test]
    fn exec_quoted_heredoc_to_fd_keeps_trailing_blank_lines() {
        assert_parity_in(Path::new("/tmp"), "exec 3<<'EOF'\nhello\n\n\nEOF\ncat <&3\n");
    }

    #[test]
    fn exec_quoted_named_fd_heredoc_keeps_trailing_blank_lines() {
        assert_parity_in(
            Path::new("/tmp"),
            "exec {f}<<'EOF'\nhello\n\n\nEOF\ncat <&$f\n",
        );
    }

    /// NEGATIVE CONTROL for the flag being threaded rather than the
    /// append being deleted: a GENUINE here-string still gains its
    /// trailing newline (c:4665-4666 "as if the string given was a
    /// complete command line").
    #[test]
    fn exec_herestring_to_fd_still_appends_newline() {
        assert_parity_in(
            Path::new("/tmp"),
            "exec 3<<< hello\ncat <&3 | od -An -c | tr -s ' '\n",
        );
    }

    /// An empty here-document body still OPENS fd N on a zero-byte
    /// file; it must not be diverted to the pending-stdin path (which
    /// left fd N closed and made `cat <&3` fail).
    #[test]
    fn exec_empty_heredoc_to_fd_still_opens_it() {
        assert_parity_in(Path::new("/tmp"), "exec 3<<EOF\nEOF\ncat <&3\nprint rc=$?\n");
    }

    /// The mirror image of the persistence contract: WITHOUT a bare
    /// `exec`, the fd belongs to that one command only. c:Src/exec.c:
    /// 2421-2443 parks the old contents in `save[]` — `-1` when the fd
    /// was closed beforehand (c:2422-2423) — and c:4530 `redup(save[i],
    /// i)` restores it, closing the fd again for the `-1` case
    /// (c:Src/utils.c:2047-2048).
    #[test]
    fn non_exec_heredoc_fd_does_not_leak() {
        assert_parity_in(
            Path::new("/tmp"),
            "print -r -- start 3<<EOF\nhi\nEOF\ncat <&3\nprint rc=$?\n",
        );
    }

    #[test]
    fn non_exec_herestring_fd_does_not_leak() {
        assert_parity_in(
            Path::new("/tmp"),
            "print -r -- start 3<<< hi\ncat <&3\nprint rc=$?\n",
        );
    }

    /// …and when the fd DID hold something before the command, the
    /// command-scoped here-document must hand it back, not close it.
    #[test]
    fn non_exec_heredoc_fd_restores_previous_contents() {
        let d = tdir();
        std::fs::write(d.path().join("outer.txt"), "outer\n").expect("write");
        assert_parity_in(
            d.path(),
            "exec 3< outer.txt\nprint -r -- start 3<<EOF\nhi\nEOF\ncat <&3\n",
        );
    }
}

/// A `return` out of a compound command that carries a redirection must
/// restore the fd, exactly like every other exit path from `execcmd_exec`
/// (c:Src/exec.c:4364 `fixfds(save)`). zshrs used to jump past the scope's
/// restore, so the CALLER inherited the callee's redirected fd.
///
/// This is what broke gitstatus under zshrs: the daemon points fd 0 at its
/// request FIFO, then sources `gitstatus/install`, whose
/// `_gitstatus_install_main` returns out of `while … done <install.info`.
/// fd 0 stayed on install.info, `gitstatusd` read EOF and exited, and
/// powerlevel10k lost `VCS_STATUS_REMOTE_URL` (and with it the per-forge
/// VCS icon). BUGS.md #1089.
mod return_restores_redirect {
    use super::*;

    /// Fixture: `inner.txt` is what the redirected compound command reads,
    /// `outer.txt` is what the enclosing scope's fd 0 must still be on
    /// after the function returns.
    const SETUP: &str = "printf 'IN1\\nIN2\\n' > inner.txt\n\
                         printf 'OUTER\\n' > outer.txt\n";

    fn assert_fd0_restored(body: &str) {
        let d = tdir();
        assert_parity_in(d.path(), &format!("{SETUP}{body}"));
    }

    #[test]
    fn return_from_while_with_redirect() {
        assert_fd0_restored(
            "f() { local l; while IFS= read -r l; do return 0; done < inner.txt }\n\
             { f; print -n 'fd0='; cat } < outer.txt\n",
        );
    }

    #[test]
    fn return_from_brace_group_with_redirect() {
        assert_fd0_restored(
            "f() { local l; { read -r l; return 0 } < inner.txt }\n\
             { f; print -n 'fd0='; cat } < outer.txt\n",
        );
    }

    #[test]
    fn return_from_for_with_redirect() {
        assert_fd0_restored(
            "f() { local l; for l in a b; do return 0; done < inner.txt }\n\
             { f; print -n 'fd0='; cat } < outer.txt\n",
        );
    }

    #[test]
    fn return_from_if_with_redirect() {
        assert_fd0_restored(
            "f() { if true; then return 0; fi < inner.txt }\n\
             { f; print -n 'fd0='; cat } < outer.txt\n",
        );
    }

    /// The gitstatus shape exactly: the redirected loop with the early
    /// `return` lives in a SOURCED file, and the caller checks fd 0 after
    /// `source` comes back.
    #[test]
    fn return_from_redirected_loop_in_sourced_file() {
        assert_fd0_restored(
            "print -r -- '_main() { local l; while IFS= read -r l; do return 0; done < inner.txt }\n\
             _main' > inst.sh\n\
             { source ./inst.sh; print -n 'fd0='; cat } < outer.txt\n",
        );
    }

    /// Regression guard: plain loop exit (no `return`) already restored the
    /// fd and must keep doing so.
    #[test]
    fn loop_falls_off_end_with_redirect() {
        assert_fd0_restored(
            "f() { local l; while IFS= read -r l; do :; done < inner.txt }\n\
             { f; print -n 'fd0='; cat } < outer.txt\n",
        );
    }

    /// Regression guard: a redirect the function opens with bare `exec` is
    /// deliberately NOT restored (c:Src/exec.c:3978-3986 nullexec==1).
    #[test]
    fn bare_exec_redirect_still_survives_the_function() {
        assert_fd0_restored(
            "f() { exec < inner.txt }\n\
             { f; print -n 'fd0='; cat } < outer.txt\n",
        );
    }
}
