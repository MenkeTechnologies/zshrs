//! Miscellaneous builtin parity tests — true/false/:/exit/shift/let/getopts/source/exec.

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

mod true_false {
    use super::*;

    #[test]
    fn true_exits_zero() {
        assert_parity("true; echo $?");
    }

    #[test]
    fn false_exits_one() {
        assert_parity("false; echo $?");
    }

    #[test]
    fn colon_null_exits_zero() {
        assert_parity(": ; echo $?");
    }

    #[test]
    fn colon_with_args_ignores_them() {
        assert_parity(": ignored args; echo $?");
    }

    /// `true` and `false` chained correctly in if.
    #[test]
    fn if_true_then_branch() {
        assert_parity("if true; then echo yes; else echo no; fi");
    }

    #[test]
    fn if_false_else_branch() {
        assert_parity("if false; then echo yes; else echo no; fi");
    }
}

mod exit_builtin {
    use super::*;

    #[test]
    fn exit_zero_default() {
        assert_parity("(exit); echo $?");
    }

    #[test]
    fn exit_explicit_code() {
        assert_parity("(exit 42); echo $?");
    }

    /// exit with too-large code wraps to 0..255.
    #[test]
    fn exit_256_wraps_to_zero() {
        assert_parity("(exit 256); echo $?");
    }

    #[test]
    fn exit_in_function() {
        assert_parity("f() { exit 7; }; (f); echo $?");
    }
}

mod shift_builtin {
    use super::*;

    #[test]
    fn shift_drops_one_positional() {
        assert_parity("set -- a b c d; shift; echo $1 $2 $3");
    }

    #[test]
    fn shift_two_drops_two() {
        assert_parity("set -- a b c d; shift 2; echo $1 $2");
    }

    #[test]
    fn shift_more_than_count_errors() {
        assert_parity("set -- a b; shift 5 2>/dev/null; echo $?");
    }

    #[test]
    fn shift_zero_is_noop() {
        assert_parity("set -- a b c; shift 0; echo $1 $2 $3");
    }
}

mod let_builtin {
    use super::*;

    #[test]
    fn let_assignment_and_read() {
        assert_parity(r#"let "X=5+3"; echo $X"#);
    }

    #[test]
    fn let_returns_zero_on_truthy() {
        assert_parity(r#"let "1"; echo $?"#);
    }

    #[test]
    fn let_returns_one_on_falsy() {
        assert_parity(r#"let "0"; echo $?"#);
    }

    #[test]
    fn let_with_multiple_expressions() {
        assert_parity(r#"let "A=1" "B=2" "C=A+B"; echo $C"#);
    }
}

mod set_positional {
    use super::*;

    #[test]
    fn set_dashdash_replaces_positional() {
        assert_parity("set -- new args; echo $#; echo $1 $2");
    }

    #[test]
    fn set_dashdash_empty_clears_positional() {
        assert_parity("set -- a b; set --; echo $#");
    }
}

mod getopts {
    use super::*;

    /// Basic getopts loop with `-a` / `-b` flags + arg.
    #[test]
    fn getopts_simple_flag_loop() {
        assert_parity(
            r#"
set -- -a -b val -c
while getopts "abc" opt; do
  echo opt=$opt
done
"#,
        );
    }

    #[test]
    fn getopts_with_arg() {
        assert_parity(
            r#"
set -- -f filename
while getopts "f:" opt; do
  echo opt=$opt arg=$OPTARG
done
"#,
        );
    }

    #[test]
    fn getopts_optind_after_loop() {
        assert_parity(
            r#"
set -- -a -b
while getopts "ab" opt; do :; done
echo OPTIND=$OPTIND
"#,
        );
    }

    #[test]
    fn getopts_unknown_option_emits_question_mark() {
        assert_parity(
            r#"
set -- -X
while getopts "ab" opt 2>/dev/null; do
  echo opt=$opt
done
"#,
        );
    }
}

mod source_dot {
    use super::*;

    /// `source` runs a script in current shell.
    #[test]
    fn source_runs_script_in_current_shell() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("snippet.sh");
        std::fs::write(&p, "echo from-source\n").unwrap();
        let script = format!(r#"source "{}""#, p.display());
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stdout, r.stdout);
    }

    /// `.` is alias for `source`.
    #[test]
    fn dot_is_alias_for_source() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("snippet.sh");
        std::fs::write(&p, "echo from-dot\n").unwrap();
        let script = format!(r#". "{}""#, p.display());
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stdout, r.stdout);
    }

    /// Sourced script sees + modifies current shell's vars.
    #[test]
    fn source_can_modify_parent_vars() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("set.sh");
        std::fs::write(&p, "X=from_sourced\n").unwrap();
        let script = format!(r#"X=before; source "{}"; echo $X"#, p.display());
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stdout, r.stdout);
    }
}

mod which_type {
    use super::*;

    /// `type true` should identify true as a builtin or executable.
    #[test]
    fn type_true_classifies() {
        assert_parity(r#"type true | head -1"#);
    }

    /// `type nonexistent_xyz` errors.
    #[test]
    fn type_unknown_command_errors() {
        assert_parity(r#"type nonexistent_xyz_42 2>/dev/null; echo $?"#);
    }
}
