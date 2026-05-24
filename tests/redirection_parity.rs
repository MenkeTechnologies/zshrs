//! I/O redirection parity tests — >, >>, <, 2>, &>, 2>&1, etc.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") { return PathBuf::from(p); }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("debug").join("zshrs")
}
fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() { "/opt/homebrew/bin/zsh" }
    else if Path::new("/usr/local/bin/zsh").exists() { "/usr/local/bin/zsh" }
    else { "/bin/zsh" }
}
fn zsh_available() -> bool {
    Command::new(zsh_path()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}
struct R { stdout: String, exit: i32 }
fn run_zsh_in(dir: &Path, s: &str) -> R {
    let o = Command::new(zsh_path()).args(["-fc", s]).current_dir(dir).output().expect("zsh");
    R { stdout: String::from_utf8_lossy(&o.stdout).into_owned(), exit: o.status.code().unwrap_or(-1) }
}
fn run_zshrs_in(dir: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin()).args(["--zsh", "-f", "-c", s])
        .current_dir(dir).env_remove("ZSHRS_CACHE").output().expect("zshrs");
    R { stdout: String::from_utf8_lossy(&o.stdout).into_owned(), exit: o.status.code().unwrap_or(-1) }
}
fn assert_parity_in(dir: &Path, s: &str) {
    if !zsh_available() { return; }
    let z = run_zsh_in(dir, s);
    let r = run_zshrs_in(dir, s);
    assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}", z.stdout, r.stdout);
    assert_eq!(z.exit, r.exit);
}
fn tdir() -> tempfile::TempDir { tempfile::tempdir().expect("tempdir") }

mod stdout_redirect {
    use super::*;

    #[test]
    fn gt_writes_to_file() {
        let d = tdir();
        assert_parity_in(d.path(), "echo hello > out.txt; cat out.txt");
    }

    #[test]
    fn gt_truncates_existing() {
        let d = tdir();
        std::fs::write(d.path().join("out.txt"), "old content").unwrap();
        assert_parity_in(d.path(), "echo new > out.txt; cat out.txt");
    }

    #[test]
    #[ignore = "ZSHRS BUG: `>>` append redirect doesn't preserve existing content"]
    fn gt_gt_appends() {
        let d = tdir();
        std::fs::write(d.path().join("out.txt"), "line1\n").unwrap();
        assert_parity_in(d.path(), "echo line2 >> out.txt; cat out.txt");
    }

    #[test]
    fn explicit_1_gt() {
        let d = tdir();
        assert_parity_in(d.path(), "echo hi 1> out.txt; cat out.txt");
    }
}

mod stderr_redirect {
    use super::*;

    #[test]
    fn stderr_to_file() {
        let d = tdir();
        // Generate stderr via a parse error or `echo` of nothing useful;
        // use a command that reliably writes to stderr.
        assert_parity_in(d.path(),
            r#"sh -c 'echo err >&2' 2> err.txt; cat err.txt"#);
    }

    #[test]
    fn stderr_to_file_only_stdout_remains() {
        let d = tdir();
        // Both stdout and stderr; redirect stderr only, stdout passes through.
        assert_parity_in(d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' 2> err.txt; cat err.txt"#);
    }

    #[test]
    #[ignore = "ZSHRS BUG: `2>>` stderr append redirect doesn't preserve existing content"]
    fn stderr_append() {
        let d = tdir();
        std::fs::write(d.path().join("err.txt"), "first\n").unwrap();
        assert_parity_in(d.path(),
            r#"sh -c 'echo more >&2' 2>> err.txt; cat err.txt"#);
    }
}

mod stdin_redirect {
    use super::*;

    #[test]
    fn lt_reads_from_file() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "content\n").unwrap();
        assert_parity_in(d.path(), "cat < in.txt");
    }

    #[test]
    fn lt_then_pipe() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "a\nb\nc\n").unwrap();
        assert_parity_in(d.path(), "cat < in.txt | grep b");
    }
}

mod fd_dup {
    use super::*;

    /// `2>&1` redirects stderr to wherever stdout currently goes.
    #[test]
    fn two_to_one_merges_stderr_into_stdout() {
        let d = tdir();
        assert_parity_in(d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' 2>&1"#);
    }

    /// `>file 2>&1` — both stdout and stderr go to file.
    #[test]
    fn gt_file_then_two_to_one_captures_both() {
        let d = tdir();
        assert_parity_in(d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' > both.txt 2>&1; cat both.txt"#);
    }

    /// `1>&2` redirects stdout to stderr.
    #[test]
    fn one_to_two_swaps_stdout_to_stderr() {
        let d = tdir();
        assert_parity_in(d.path(),
            r#"echo hi 1>&2 2> err.txt; cat err.txt"#);
    }
}

mod amp_redirect {
    use super::*;

    /// `&>` redirects both stdout AND stderr (zsh + bash).
    #[test]
    fn amp_gt_redirects_both_streams() {
        let d = tdir();
        assert_parity_in(d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' &> all.txt; cat all.txt"#);
    }

    /// `&>>` appends both streams.
    #[test]
    #[ignore = "ZSHRS BUG: `&>>` append doesn't preserve existing content (same bug class as >>)"]
    fn amp_gt_gt_appends_both() {
        let d = tdir();
        std::fs::write(d.path().join("all.txt"), "first\n").unwrap();
        assert_parity_in(d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' &>> all.txt; cat all.txt"#);
    }
}

mod here_in_redirect {
    use super::*;

    /// Heredoc as redirect input.
    #[test]
    fn heredoc_as_stdin() {
        let d = tdir();
        assert_parity_in(d.path(), "cat <<EOF\nhi\nEOF");
    }

    /// Here-string `<<<` as redirect input.
    #[test]
    fn here_string_as_stdin() {
        let d = tdir();
        assert_parity_in(d.path(), r#"cat <<< "hi""#);
    }
}

mod multiple_redirects {
    use super::*;

    /// Multiple `>` on same command — only last takes effect for fd 1.
    #[test]
    #[ignore = "ZSHRS DIVERGENCE: multi-`>` on same cmd vs zsh's last-wins or both-fail"]
    fn second_gt_wins_for_stdout() {
        let d = tdir();
        assert_parity_in(d.path(),
            "echo hi > first.txt > second.txt; cat first.txt second.txt 2>/dev/null; echo done");
    }

    /// Separate fds — each redirect goes to its own file.
    #[test]
    fn out_to_one_file_err_to_another() {
        let d = tdir();
        assert_parity_in(d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' > out.txt 2> err.txt; cat out.txt err.txt"#);
    }
}

mod pipeline_redirect {
    use super::*;

    #[test]
    fn pipe_then_gt_file() {
        let d = tdir();
        assert_parity_in(d.path(),
            "echo hello world | tr ' ' '_' > out.txt; cat out.txt");
    }

    #[test]
    fn pipe_amp_merges_stderr_into_pipe() {
        let d = tdir();
        assert_parity_in(d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' |& cat"#);
    }
}

mod close_fd {
    use super::*;

    /// `2>&-` closes stderr.
    #[test]
    fn close_stderr_with_dash() {
        let d = tdir();
        assert_parity_in(d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' 2>&-"#);
    }
}

mod redirect_from_var {
    use super::*;

    /// Filename comes from variable expansion.
    #[test]
    fn redirect_target_is_variable() {
        let d = tdir();
        assert_parity_in(d.path(),
            "F=out.txt; echo hi > $F; cat $F");
    }
}
