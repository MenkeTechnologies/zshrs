//! I/O redirection parity tests — >, >>, <, 2>, &>, 2>&1, etc.

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
fn run_zsh_in(dir: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(dir)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs_in(dir: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(dir)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity_in(dir: &Path, s: &str) {
    if !zsh_available() {
        return;
    }
    // Snapshot every file in `dir` BEFORE the first shell runs so the
    // second shell sees the same starting state. Without this, redirect
    // tests that pre-create files (`echo line1 > out.txt` then
    // `assert_parity_in`) see zsh mutate the file first, then zshrs run
    // against zsh's already-modified state. Cumulative-append redirects
    // (`>>`, `&>>`, `2>>`) then look broken even when both shells
    // perform identical operations.
    let snapshot: Vec<(PathBuf, Vec<u8>)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter_map(|e| {
                    let p = e.path();
                    std::fs::read(&p).ok().map(|b| (p, b))
                })
                .collect()
        })
        .unwrap_or_default();
    let z = run_zsh_in(dir, s);
    // Restore the snapshot so zshrs starts from the same state.
    for (p, b) in &snapshot {
        let _ = std::fs::write(p, b);
    }
    let r = run_zshrs_in(dir, s);
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
        assert_parity_in(d.path(), r#"sh -c 'echo err >&2' 2> err.txt; cat err.txt"#);
    }

    #[test]
    fn stderr_to_file_only_stdout_remains() {
        let d = tdir();
        // Both stdout and stderr; redirect stderr only, stdout passes through.
        assert_parity_in(
            d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' 2> err.txt; cat err.txt"#,
        );
    }

    #[test]
    fn stderr_append() {
        let d = tdir();
        std::fs::write(d.path().join("err.txt"), "first\n").unwrap();
        assert_parity_in(
            d.path(),
            r#"sh -c 'echo more >&2' 2>> err.txt; cat err.txt"#,
        );
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
        assert_parity_in(d.path(), r#"sh -c 'echo OUT; echo ERR >&2' 2>&1"#);
    }

    /// `>file 2>&1` — both stdout and stderr go to file.
    #[test]
    fn gt_file_then_two_to_one_captures_both() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' > both.txt 2>&1; cat both.txt"#,
        );
    }

    /// `1>&2` redirects stdout to stderr.
    #[test]
    fn one_to_two_swaps_stdout_to_stderr() {
        let d = tdir();
        assert_parity_in(d.path(), r#"echo hi 1>&2 2> err.txt; cat err.txt"#);
    }
}

mod amp_redirect {
    use super::*;

    /// `&>` redirects both stdout AND stderr (zsh + bash).
    #[test]
    fn amp_gt_redirects_both_streams() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' &> all.txt; cat all.txt"#,
        );
    }

    /// `&>>` appends both streams.
    #[test]
    fn amp_gt_gt_appends_both() {
        let d = tdir();
        std::fs::write(d.path().join("all.txt"), "first\n").unwrap();
        assert_parity_in(
            d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' &>> all.txt; cat all.txt"#,
        );
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
        assert_parity_in(
            d.path(),
            "echo hi > first.txt > second.txt; cat first.txt second.txt 2>/dev/null; echo done",
        );
    }

    /// Separate fds — each redirect goes to its own file.
    #[test]
    fn out_to_one_file_err_to_another() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            r#"sh -c 'echo OUT; echo ERR >&2' > out.txt 2> err.txt; cat out.txt err.txt"#,
        );
    }
}

mod pipeline_redirect {
    use super::*;

    #[test]
    fn pipe_then_gt_file() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            "echo hello world | tr ' ' '_' > out.txt; cat out.txt",
        );
    }

    #[test]
    fn pipe_amp_merges_stderr_into_pipe() {
        let d = tdir();
        assert_parity_in(d.path(), r#"sh -c 'echo OUT; echo ERR >&2' |& cat"#);
    }
}

mod close_fd {
    use super::*;

    /// `2>&-` closes stderr.
    #[test]
    fn close_stderr_with_dash() {
        let d = tdir();
        assert_parity_in(d.path(), r#"sh -c 'echo OUT; echo ERR >&2' 2>&-"#);
    }
}

mod redirect_from_var {
    use super::*;

    /// Filename comes from variable expansion.
    #[test]
    fn redirect_target_is_variable() {
        let d = tdir();
        assert_parity_in(d.path(), "F=out.txt; echo hi > $F; cat $F");
    }
}

/// MULTIOS semantics (Bug #36) — c:Src/exec.c:2391-2480 addfd +
/// closemn tee/cat, c:Src/glob.c:2150-2207 xpandredir.
mod multios {
    use super::*;

    /// `> a > b` tees stdout to both files.
    #[test]
    fn tee_two_files() {
        let d = tdir();
        assert_parity_in(d.path(), "print x > a > b; cat a b");
    }

    /// Mixed modes: `> a >> b` truncates a, appends b.
    #[test]
    fn tee_mixed_write_append() {
        let d = tdir();
        std::fs::write(d.path().join("b"), "pre\n").unwrap();
        assert_parity_in(d.path(), "print x > a >> b; cat a; cat b");
    }

    /// `< a < b` concatenates inputs in redirect order.
    #[test]
    fn input_concat_order() {
        let d = tdir();
        std::fs::write(d.path().join("a"), "first\n").unwrap();
        std::fs::write(d.path().join("b"), "second\n").unwrap();
        assert_parity_in(d.path(), "cat < a < b");
    }

    /// NO_MULTIOS output: last redirect wins; earlier files are
    /// still created/truncated (c:2418 `unset(MULTIOS)` replace arm).
    #[test]
    fn nomultios_last_write_wins() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            r#"unsetopt multios; print x > a > b; ls; echo "a:[$(cat a)] b:[$(cat b)]""#,
        );
    }

    /// NO_MULTIOS input: only the last source is read.
    #[test]
    fn nomultios_last_read_wins() {
        let d = tdir();
        std::fs::write(d.path().join("a"), "one\n").unwrap();
        std::fs::write(d.path().join("b"), "two\n").unwrap();
        assert_parity_in(d.path(), "unsetopt multios; cat < a < b");
    }

    /// `>&1 > f` — the dup of the ORIGINAL stdout joins the multio
    /// (c:Src/exec.c:3895-3917 REDIR_MERGEOUT + addfd).
    #[test]
    fn dup_then_file_tees_original_stdout() {
        let d = tdir();
        assert_parity_in(d.path(), r#"print x >&1 > f; echo "f:[$(cat f)]""#);
    }

    /// `> f >&1` — the dup resolves AFTER fd1 was replaced by f, so
    /// f receives the stream twice.
    #[test]
    fn file_then_dup_doubles_into_file() {
        let d = tdir();
        assert_parity_in(d.path(), r#"print x > f >&1; echo "f:[$(cat f)]""#);
    }

    /// `>&1 > f | cat` — the pipeline pipe seeds the multio
    /// (c:Src/exec.c:3722-3724), so the pipe receives the stream
    /// twice (seed + dup) and f once.
    #[test]
    fn dup_and_file_inside_pipeline() {
        let d = tdir();
        assert_parity_in(d.path(), r#"print x >&1 > f | cat; echo "f:[$(cat f)]""#);
    }

    /// `>&1 >&1` — two self-dups double the stream.
    #[test]
    fn double_self_dup() {
        let d = tdir();
        assert_parity_in(d.path(), "print x >&1 >&1");
    }

    /// fd-2 multio with a dup member: `2>&1 2>f` sends stderr to
    /// stdout AND the file.
    #[test]
    fn stderr_dup_plus_file() {
        let d = tdir();
        assert_parity_in(d.path(), r#"print -u2 e 2>&1 2>f; echo "f:[$(cat f)]""#);
    }

    /// fd-2 multio, two files: `2> e1 2> e2`.
    #[test]
    fn stderr_tee_two_files() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            r#"print -u2 err 2> e1 2> e2; echo "e1:[$(cat e1)] e2:[$(cat e2)]""#,
        );
    }

    /// Glob redirect target with two matches writes both files
    /// (c:Src/glob.c:2195-2203 — one redirect per match, one multio).
    #[test]
    fn glob_write_two_matches() {
        let d = tdir();
        std::fs::write(d.path().join("g1.txt"), "").unwrap();
        std::fs::write(d.path().join("g2.txt"), "").unwrap();
        assert_parity_in(d.path(), "echo hi > *.txt; cat g1.txt g2.txt");
    }

    /// Glob input source with two matches concatenates both.
    #[test]
    fn glob_read_two_matches() {
        let d = tdir();
        std::fs::write(d.path().join("g1.txt"), "aa").unwrap();
        std::fs::write(d.path().join("g2.txt"), "bbb").unwrap();
        assert_parity_in(d.path(), "wc -c < *.txt");
    }

    /// NO_MULTIOS suppresses redirect-target globbing entirely
    /// (c:Src/glob.c:2161-2167 PREFORK_SINGLE): `> *.txt` creates
    /// the literal file `*.txt`.
    #[test]
    fn nomultios_glob_stays_literal() {
        let d = tdir();
        std::fs::write(d.path().join("g1.txt"), "").unwrap();
        std::fs::write(d.path().join("g2.txt"), "").unwrap();
        assert_parity_in(d.path(), "unsetopt multios; echo hi > *.txt; ls");
    }

    /// noclobber failure on a multio member aborts the rest of the
    /// redirect list — `b` is never created (c:Src/exec.c execerr).
    #[test]
    fn noclobber_aborts_multio() {
        let d = tdir();
        std::fs::write(d.path().join("a"), "").unwrap();
        assert_parity_in(
            d.path(),
            "setopt noclobber; print x > a > b; echo st=$?; ls",
        );
    }

    /// Three-way fan-out.
    #[test]
    fn tee_three_files() {
        let d = tdir();
        assert_parity_in(d.path(), "print x > a > b > c; cat a b c");
    }

    /// Exit status is the command's own, not the splitter's.
    #[test]
    fn exit_status_after_tee() {
        let d = tdir();
        assert_parity_in(d.path(), "print x > a > b; echo $?");
    }
}
