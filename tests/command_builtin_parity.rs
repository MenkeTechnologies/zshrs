//! `command` builtin parity: -v, -V, -p, function/alias bypass.

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

mod basic_exec {
    use super::*;

    /// `command echo hi` runs the echo command.
    #[test]
    fn command_runs_external_or_builtin() {
        assert_parity(r#"command echo hi"#);
    }

    /// `command false` exits 1.
    #[test]
    fn command_false_exit_1() {
        assert_parity(r#"command false; echo $?"#);
    }

    /// `command true` exits 0.
    #[test]
    fn command_true_exit_0() {
        assert_parity(r#"command true; echo $?"#);
    }
}

mod bypass_function {
    use super::*;

    /// Define function named "ls" → `command ls` bypasses it.
    #[test]
    fn command_bypasses_function_with_same_name() {
        assert_parity(
            r#"
ls() { echo "function-version"; }
command ls /tmp >/dev/null 2>&1
echo exit=$?
"#,
        );
    }

    /// Without `command`, function takes precedence.
    #[test]
    fn without_command_function_used() {
        assert_parity(
            r#"
ls() { echo "function-version"; }
ls
"#,
        );
    }

    /// Function shadows builtin → command bypasses.
    #[test]
    fn command_bypasses_function_that_shadows_builtin() {
        assert_parity(
            r#"
print() { echo "function-print"; }
command print "real-print"
"#,
        );
    }
}

mod dash_v {
    use super::*;

    /// `command -v ls` prints path/builtin info.
    #[test]
    fn dash_v_external_command() {
        assert_parity(r#"command -v ls"#);
    }

    /// `command -v cd` (builtin) reports "cd".
    #[test]
    fn dash_v_builtin() {
        assert_parity(r#"command -v cd"#);
    }

    /// `command -v nonexistent` → exit 1, empty.
    #[test]
    fn dash_v_unknown_exit_1() {
        assert_parity(r#"command -v nonexistent_xyz_42 2>/dev/null; echo exit=$?"#);
    }

    /// `command -v` for alias prints alias definition.
    #[test]
    fn dash_v_alias_prints_definition() {
        assert_parity(r#"alias myh='echo hi'; command -v myh"#);
    }

    /// `command -v` for function prints function name (or definition).
    #[test]
    fn dash_v_function() {
        assert_parity(r#"f() { :; }; command -v f"#);
    }
}

mod dash_V {
    use super::*;

    /// `command -V ls` prints verbose info.
    #[test]
    fn dash_V_external_command() {
        assert_parity(r#"command -V ls 2>/dev/null | head -1"#);
    }

    /// `command -V cd` prints "cd is a builtin" or similar.
    #[test]
    fn dash_V_builtin() {
        assert_parity(r#"command -V cd"#);
    }

    /// `command -V nonexistent` → error.
    #[test]
    fn dash_V_unknown_exit_nonzero() {
        assert_parity(r#"command -V nonexistent_xyz_42 2>/dev/null; echo exit=$?"#);
    }
}

mod dash_p {
    use super::*;

    /// `command -p` uses default PATH for lookup.
    #[test]
    fn dash_p_resets_path_search() {
        assert_parity(r#"PATH=""; command -p ls /tmp >/dev/null 2>&1; echo exit=$?"#);
    }
}

mod multi_args {
    use super::*;

    /// Multiple args to command.
    #[test]
    fn command_with_multiple_args() {
        assert_parity(r#"command echo a b c"#);
    }

    /// Command + redirect.
    #[test]
    fn command_with_redirect() {
        assert_parity(r#"command echo hello > /dev/null; echo done"#);
    }

    /// command in a pipeline.
    #[test]
    fn command_in_pipeline() {
        assert_parity(r#"echo foobar | command grep foo"#);
    }
}

mod command_command {
    use super::*;

    /// `command command echo` works recursively.
    #[test]
    fn command_command_echo() {
        assert_parity(r#"command command echo hi"#);
    }
}

mod command_keywords {
    use super::*;

    /// `command` can run a keyword? In bash/zsh, keywords like `if`
    /// cannot be run via `command` — `command if true; then ...; fi`
    /// fails. Pin behavior.
    #[test]
    fn command_keyword_if_errors() {
        assert_parity(r#"command if 2>/dev/null; echo exit=$?"#);
    }
}

mod exec_path_resolution {
    use super::*;

    /// Path-based command lookup.
    #[test]
    fn command_with_full_path() {
        assert_parity(r#"command /bin/echo hello"#);
    }

    /// Relative path.
    #[test]
    fn command_with_relative_path() {
        assert_parity(r#"cd /tmp && command ./../bin/echo hi 2>/dev/null; echo exit=$?"#);
    }
}
