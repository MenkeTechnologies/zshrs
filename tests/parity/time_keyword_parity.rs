//! `time` keyword parity tests.
//!
//! NB: `time` output goes to stderr and contains wall/user/sys
//! durations that vary per run. We test the structure & exit codes.

#![allow(non_snake_case)]

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
    stderr: String,
    exit: i32,
}
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
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
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod basic_exit_codes {
    use super::*;

    /// time + true exits 0.
    #[test]
    fn time_true_exit_0() {
        assert_parity(r#"time true 2>/dev/null; echo $?"#);
    }

    /// time + false exits 1.
    #[test]
    fn time_false_exit_1() {
        assert_parity(r#"time false 2>/dev/null; echo $?"#);
    }

    /// time of (exit 42) exits 42.
    #[test]
    fn time_explicit_exit_propagates() {
        assert_parity(r#"time (exit 42) 2>/dev/null; echo $?"#);
    }
}

mod time_stdout_passthrough {
    use super::*;

    /// `time` doesn't touch the timed command's stdout.
    #[test]
    fn time_echo_stdout_unchanged() {
        assert_parity(r#"time echo hello 2>/dev/null"#);
    }

    /// `time` with a pipe — stdout of last cmd flows through.
    #[test]
    fn time_pipeline_stdout() {
        assert_parity(r#"time { echo hello | tr a-z A-Z } 2>/dev/null"#);
    }
}

mod time_block {
    use super::*;

    /// `time { compound }` works.
    #[test]
    fn time_compound_block() {
        assert_parity(r#"time { true; true; true } 2>/dev/null; echo $?"#);
    }

    /// `time ( subshell )` works.
    #[test]
    fn time_subshell_block() {
        assert_parity(r#"time ( true ) 2>/dev/null; echo $?"#);
    }
}

mod time_stderr_present {
    use super::*;

    /// `time` writes its summary on stderr only when the inner
    /// command actually consumed CPU (zsh reads rusage from waitpid;
    /// builtins produce zero usage and zsh prints nothing). Both
    /// shells must agree on whether stderr is empty or not.
    #[test]
    fn time_emits_some_stderr_in_zsh() {
        if !zsh_available() {
            return;
        }
        let z = run_zsh(r#"time true"#);
        let r = run_zshrs(r#"time true"#);
        // Parity: both empty (builtin) or both non-empty.
        assert_eq!(
            z.stderr.is_empty(),
            r.stderr.is_empty(),
            "time stderr emission differs:\nzsh: {:?}\nzshrs: {:?}",
            z.stderr,
            r.stderr
        );
    }

    /// stderr from `time` contains "total" or similar marker
    /// (zsh default format includes ' total').
    #[test]
    fn time_default_stderr_format_has_known_marker() {
        if !zsh_available() {
            return;
        }
        let z = run_zsh(r#"time true"#);
        let r = run_zshrs(r#"time true"#);
        // zsh default ends with 'total'; check both shells include it
        // (or both don't — for `time true` zsh emits nothing).
        let z_has_total = z.stderr.contains("total");
        let r_has_total = r.stderr.contains("total");
        assert_eq!(
            z_has_total, r_has_total,
            "time output format differs:\nzsh stderr: {:?}\nzshrs stderr: {:?}",
            z.stderr, r.stderr
        );
    }
}

mod timefmt {
    use super::*;

    /// Custom `TIMEFMT='%J'` → output is just the command name.
    #[test]
    fn timefmt_J_just_command_name() {
        if !zsh_available() {
            return;
        }
        let z = run_zsh(r#"TIMEFMT='%J'; time true"#);
        let r = run_zshrs(r#"TIMEFMT='%J'; time true"#);
        // Both should produce stderr ending with "true\n" (or similar).
        let z_trim = z.stderr.trim();
        let r_trim = r.stderr.trim();
        assert_eq!(
            z_trim, r_trim,
            "TIMEFMT=%J format mismatch:\nzsh: {z_trim:?}\nzshrs: {r_trim:?}"
        );
    }
}

mod time_in_pipeline {
    use super::*;

    /// time with a pipeline.
    #[test]
    fn time_real_pipeline() {
        assert_parity(r#"time { echo hello | cat | cat } 2>/dev/null"#);
    }

    /// `time` of background job: both shells write the timing line
    /// to stderr (assert_parity ignores stderr) and produce empty
    /// stdout. The original "timing-sensitive" worry doesn't bite
    /// because we only compare stdout.
    #[test]
    fn time_backgrounded_cmd() {
        assert_parity(r#"time true & wait"#);
    }
}

mod nested_time {
    use super::*;

    /// Nested time.
    #[test]
    fn nested_time_block() {
        assert_parity(r#"time { time true 2>/dev/null } 2>/dev/null; echo $?"#);
    }
}

mod time_with_redirect {
    use super::*;

    /// time with output redirection on inner command.
    #[test]
    fn time_inner_redirect() {
        assert_parity(r#"time echo hi >/dev/null 2>/dev/null"#);
    }
}

mod time_in_subshell {
    use super::*;

    /// time inside a subshell.
    #[test]
    fn time_inside_subshell() {
        assert_parity(r#"(time true) 2>/dev/null; echo $?"#);
    }
}
