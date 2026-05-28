//! Pre-command keyword parity tests: noglob, nocorrect, builtin, command.

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

mod noglob {
    use super::*;

    /// `noglob cmd args` — skip glob expansion on the args.
    #[test]
    fn noglob_passes_literal_star() {
        assert_parity(r#"noglob echo '*.txt'"#);
    }

    #[test]
    fn noglob_passes_question_literal() {
        assert_parity(r#"noglob echo 'a?b'"#);
    }

    /// Without noglob, `*.nonexistent_xyz` would error.
    /// With noglob, passes literally.
    #[test]
    fn noglob_passes_literal_to_command() {
        let d = tempfile::tempdir().unwrap();
        assert_parity_in(d.path(), r#"noglob echo *.nonexistent_xyz_42"#);
    }

    /// noglob doesn't affect non-glob args.
    #[test]
    fn noglob_doesnt_affect_plain_args() {
        assert_parity(r#"noglob echo hello world"#);
    }
}

mod command_keyword {
    use super::*;

    /// `command cmd` bypasses aliases.
    #[test]
    fn command_bypasses_alias() {
        assert_parity(r#"alias true='false'; command true; echo $?"#);
    }

    /// `command cmd` bypasses functions.
    #[test]
    fn command_bypasses_function() {
        assert_parity(r#"true() { false; }; command true; echo $?"#);
    }

    /// `command -v cmd` is POSIX `whence`.
    #[test]
    fn command_v_resolves_external() {
        assert_parity(r#"command -v ls >/dev/null; echo $?"#);
    }

    #[test]
    fn command_v_unknown_fails() {
        assert_parity(r#"command -v nonexistent_xyz_42 2>/dev/null; echo $?"#);
    }

    /// `command -V cmd` verbose form.
    #[test]
    fn command_V_verbose_succeeds() {
        assert_parity(r#"command -V true >/dev/null; echo $?"#);
    }
}

mod builtin_keyword {
    use super::*;

    /// `builtin cmd` forces the shell-builtin (skips function/alias).
    #[test]
    fn builtin_keyword_runs_builtin() {
        assert_parity(r#"echo() { :; }; builtin echo "true echo"; unset -f echo"#);
    }

    /// `builtin nonexistent` errors.
    #[test]
    fn builtin_nonexistent_errors() {
        assert_parity(r#"builtin nonexistent_xyz_42 2>/dev/null; echo $?"#);
    }
}

mod nocorrect {
    use super::*;

    /// `nocorrect cmd` skips spelling correction prompt.
    /// In -c mode no prompt happens anyway; pin still-runs.
    #[test]
    fn nocorrect_runs_command_normally() {
        assert_parity(r#"nocorrect echo hi"#);
    }
}

mod combined {
    use super::*;

    /// `noglob command echo *.txt` — both keywords stack.
    #[test]
    fn noglob_then_command_stacks() {
        assert_parity(r#"alias echo='/bin/cat'; noglob command echo '*.txt'"#);
    }
}

mod with_redirects {
    use super::*;

    /// `noglob cmd > file` — noglob applies to cmd args, not redirect target.
    #[test]
    fn noglob_with_redirect() {
        let d = tempfile::tempdir().unwrap();
        let script = "noglob echo '*.txt' > out.txt; cat out.txt";
        assert_parity_in(d.path(), script);
    }
}

mod precedence {
    use super::*;

    /// `builtin command echo` — both keywords nested, runs the builtin echo.
    #[test]
    fn builtin_then_command_runs_builtin_echo() {
        assert_parity(r#"builtin command echo hi"#);
    }
}
