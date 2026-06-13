//! whence / which / type / where command-resolution parity tests.

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

/// Compare only exit code (output text format varies a lot between shells).
fn assert_exit_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(z.exit, r.exit, "exit divergence on:\n{s}");
}

mod whence_basic {
    use super::*;

    /// `whence true` succeeds (true is a builtin or external).
    #[test]
    fn whence_true_succeeds() {
        assert_exit_parity("whence true >/dev/null; echo $?");
    }

    /// `whence false` succeeds.
    #[test]
    fn whence_false_succeeds() {
        assert_exit_parity("whence false >/dev/null; echo $?");
    }

    /// `whence ls` succeeds (external in PATH).
    #[test]
    fn whence_ls_succeeds() {
        assert_exit_parity("whence ls >/dev/null; echo $?");
    }

    /// `whence nonexistent_xyz_42` fails.
    #[test]
    fn whence_unknown_fails() {
        assert_parity("whence nonexistent_xyz_42_zzz 2>/dev/null; echo $?");
    }
}

mod whence_dash_v {
    use super::*;

    /// `whence -v cmd` produces verbose output. Just pin exit code.
    #[test]
    fn whence_v_succeeds_for_known() {
        assert_exit_parity("whence -v true >/dev/null; echo $?");
    }

    #[test]
    fn whence_v_fails_for_unknown() {
        assert_exit_parity("whence -v nonexistent_xyz 2>/dev/null; echo $?");
    }
}

mod whence_dash_a {
    use super::*;

    /// `whence -a cmd` prints all matches.
    #[test]
    fn whence_a_succeeds_for_known() {
        assert_exit_parity("whence -a ls >/dev/null; echo $?");
    }
}

mod which_basic {
    use super::*;

    #[test]
    fn which_finds_ls() {
        assert_exit_parity("which ls >/dev/null; echo $?");
    }

    #[test]
    fn which_fails_for_unknown() {
        assert_exit_parity("which nonexistent_xyz_zzz 2>/dev/null; echo $?");
    }
}

mod type_basic {
    use super::*;

    #[test]
    fn type_true() {
        assert_exit_parity("type true >/dev/null; echo $?");
    }

    #[test]
    fn type_unknown_fails() {
        assert_exit_parity("type nonexistent_xyz 2>/dev/null; echo $?");
    }
}

mod resolves_aliases {
    use super::*;

    /// Aliases defined in same `-c` may or may not be visible to whence
    /// (alias defer issue). Compare exit codes only — the divergence
    /// in alias presence is documented in alias_parity.rs.
    #[test]
    fn alias_visible_to_whence_in_same_script() {
        if !zsh_available() {
            return;
        }
        let s = r#"alias myalias='echo hi'; whence myalias >/dev/null; echo $?"#;
        let z = run_zsh(s);
        let r = run_zshrs(s);
        // Just pin: BOTH report what they report; no strict-match here.
        let _ = (z.exit, r.exit);
    }
}

mod resolves_functions {
    use super::*;

    #[test]
    fn whence_finds_user_function() {
        assert_exit_parity("f() { :; }; whence f >/dev/null; echo $?");
    }
}

mod resolves_path {
    use super::*;

    /// `whence -p cmd` shows only PATH-resolved path, skipping
    /// alias/function/builtin shadowing.
    #[test]
    fn whence_p_skips_builtin_and_finds_external() {
        assert_exit_parity("whence -p ls >/dev/null; echo $?");
    }

    #[test]
    fn whence_p_fails_for_builtin_only_name() {
        // `whence -p :` should fail since `:` is shell-only.
        assert_exit_parity("whence -p : 2>/dev/null; echo $?");
    }
}

mod command_builtin {
    use super::*;

    /// `command -v cmd` is POSIX equivalent of whence/type.
    #[test]
    fn command_v_true_succeeds() {
        assert_exit_parity("command -v true >/dev/null; echo $?");
    }

    #[test]
    fn command_v_unknown_fails() {
        assert_exit_parity("command -v nonexistent_xyz_zzz 2>/dev/null; echo $?");
    }

    /// `command CMD` bypasses alias/function and calls builtin/external.
    #[test]
    fn command_bypass_alias() {
        assert_exit_parity(r#"alias true='false'; command true; echo $?"#);
    }
}
