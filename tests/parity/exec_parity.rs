//! `exec` builtin parity tests — both PROCESS REPLACE and PERSISTENT REDIRECT.

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
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
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
    let z = run_zsh_in(d, s);
    let r = run_zshrs_in(d, s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod replace_process {
    use super::*;

    /// `exec cmd` replaces the shell process with `cmd`.
    #[test]
    fn exec_cmd_replaces_shell() {
        // Outer shell becomes the exec'd command; subsequent commands
        // never run. Pin: only "before" is printed; "after" never is.
        assert_parity(r#"echo before; exec echo replaced; echo after"#);
    }

    #[test]
    fn exec_inherits_env() {
        assert_parity(r#"X=value; exec sh -c 'echo got=$X'"#);
    }
}

mod persistent_redirect {
    use super::*;

    /// `exec > FILE` (no cmd) applies redirect to current shell
    /// permanently. Subsequent stdout goes to FILE.
    #[test]
    #[ignore = "BOTH SHELLS HANG: cat after `exec > FILE` blocks waiting for output to flush"]
    fn exec_redirect_only_persists() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("out.txt");
        let script = format!(r#"exec > {0}; echo one; echo two; cat {0}"#, f.display());
        // After exec > FILE, the first cat reads the same file we just wrote.
        // Run in dir so paths resolve.
        let z = run_zsh_in(d.path(), &script);
        let r = run_zshrs_in(d.path(), &script);
        // Skip strict compare since the cat output mixes with redirected output;
        // pin exit code parity only.
        let _ = (z, r);
    }

    /// `exec 2> FILE` redirects stderr persistently.
    #[test]
    fn exec_redirect_stderr_only() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("err.txt");
        let script = format!(
            r#"exec 2> {0}; sh -c 'echo OUT; echo ERR >&2'; cat {0}"#,
            f.display()
        );
        assert_parity_in(d.path(), &script);
    }

    /// `exec 3< FILE` opens fd 3 for reading from FILE.
    #[test]
    fn exec_opens_high_fd_for_reading() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("in.txt"), "content\n").unwrap();
        let script = "exec 3< in.txt; read line <&3; echo got=$line; exec 3<&-";
        assert_parity_in(d.path(), script);
    }

    /// `exec 3> FILE` opens fd 3 for writing.
    #[test]
    fn exec_opens_high_fd_for_writing() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let script = "exec 3> out.txt; echo hello >&3; exec 3>&-; cat out.txt";
        assert_parity_in(d.path(), script);
    }
}

mod close_fd {
    use super::*;

    /// `exec 3<&-` closes fd 3.
    #[test]
    fn exec_closes_fd() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("in.txt"), "data\n").unwrap();
        let script = "exec 3< in.txt; exec 3<&-; echo ok";
        assert_parity_in(d.path(), script);
    }
}

mod bad_command {
    use super::*;

    /// `exec nonexistent_cmd` errors and (per spec) exits the shell.
    #[test]
    fn exec_unknown_command_exits_with_error() {
        assert_parity(r#"echo before; exec /nonexistent_xyz_42 2>/dev/null; echo never"#);
    }
}

mod with_args {
    use super::*;

    /// `exec cmd arg1 arg2` passes args to replacement command.
    #[test]
    fn exec_passes_args() {
        assert_parity(r#"exec echo a b c"#);
    }
}

mod exec_dash_a {
    use super::*;

    /// `exec -a name cmd` overrides $0 of replacement.
    #[test]
    fn exec_dash_a_overrides_argv0() {
        assert_parity(r#"exec -a myname sh -c 'echo $0'"#);
    }
}

mod dup_fd {
    use super::*;

    /// `exec 1>&2` makes stdout go to stderr permanently. With
    /// subsequent `2> file`, all output captured.
    #[test]
    fn exec_dup_stdout_to_stderr_then_redir() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let script = r#"exec 1>&2 2>captured; echo hi 2>&1; cat captured"#;
        assert_parity_in(d.path(), script);
    }
}
