//! Subshell `( ... )` isolation parity tests.

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

mod var_isolation {
    use super::*;

    #[test]
    fn subshell_var_doesnt_leak_out() {
        assert_parity("(X=inside; echo $X); echo [$X]");
    }

    #[test]
    fn subshell_sees_outer_var() {
        assert_parity("X=outer; (echo $X)");
    }

    #[test]
    fn subshell_modifies_locally_only() {
        assert_parity("X=outer; (X=changed; echo inside:$X); echo outside:$X");
    }

    #[test]
    fn subshell_unset_doesnt_unset_outer() {
        assert_parity("X=value; (unset X; echo inside:[$X]); echo outside:[$X]");
    }
}

mod cd_isolation {
    use super::*;

    /// `cd` in subshell doesn't affect outer pwd.
    #[test]
    fn cd_in_subshell_isolated() {
        assert_parity(r#"(cd /tmp; pwd); echo "outer: $(pwd | head -c 1)""#);
    }
}

mod set_e_isolation {
    use super::*;

    /// `set -e` in subshell doesn't leak.
    #[test]
    fn set_e_in_subshell_isolated() {
        assert_parity(r#"(set -e); echo $?; false; echo "still here: $?""#);
    }
}

mod exit_propagation {
    use super::*;

    /// `exit N` inside subshell sets subshell's exit only.
    #[test]
    fn exit_in_subshell_only_exits_subshell() {
        assert_parity("(exit 7); echo outer:$?");
    }

    /// `exit 0` from subshell still propagates as 0.
    #[test]
    fn subshell_zero_exit_propagates() {
        assert_parity("(true); echo $?");
    }

    /// Subshell exit = last command's exit.
    #[test]
    fn subshell_exit_from_last_command() {
        assert_parity("(true; false); echo $?");
    }
}

mod chained_subshells {
    use super::*;

    /// `(cmd1) && (cmd2)` runs both if first succeeds.
    #[test]
    fn chained_subshells_with_and() {
        assert_parity("(echo first) && (echo second)");
    }

    /// `(cmd1) || (cmd2)` runs second only if first fails.
    #[test]
    fn chained_subshells_with_or() {
        assert_parity("(false) || (echo fallback)");
    }
}

mod nested_subshells {
    use super::*;

    #[test]
    fn nested_two_levels() {
        assert_parity("(echo outer; (echo inner))");
    }

    #[test]
    fn nested_var_isolation_through_layers() {
        assert_parity("X=top; (X=mid; (X=bot; echo $X); echo $X); echo $X");
    }
}

mod with_redirects {
    use super::*;

    #[test]
    fn subshell_stdout_to_file() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("out.txt");
        let script = format!(r#"(echo hi) > {}; cat {}"#, f.display(), f.display());
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn subshell_stdin_from_heredoc() {
        assert_parity("(cat) <<EOF\nfrom-heredoc\nEOF");
    }
}

mod pipeline {
    use super::*;

    #[test]
    fn subshell_in_pipeline_left() {
        assert_parity("(echo a; echo b; echo c) | wc -l");
    }

    #[test]
    fn subshell_in_pipeline_right() {
        assert_parity("echo hello | (cat; echo done)");
    }

    #[test]
    fn two_subshells_in_pipeline() {
        assert_parity("(echo one; echo two) | (sort)");
    }
}

mod with_command_subst {
    use super::*;

    /// `$( ... )` is itself a subshell.
    #[test]
    fn cmdsubst_is_subshell_var_isolated() {
        assert_parity(r#"X=outer; Y=$(X=inside; echo $X); echo "Y=$Y X=$X""#);
    }
}
