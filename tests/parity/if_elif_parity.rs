//! if / elif / else conditional parity tests.

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

mod plain_if {
    use super::*;

    #[test]
    fn if_true_then_runs() {
        assert_parity("if true; then echo yes; fi");
    }

    #[test]
    fn if_false_no_output() {
        assert_parity("if false; then echo never; fi; echo done");
    }

    #[test]
    fn if_test_succeeds() {
        assert_parity(r#"if [[ "a" == "a" ]]; then echo match; fi"#);
    }

    #[test]
    fn if_arith_succeeds() {
        assert_parity("if (( 5 > 3 )); then echo big; fi");
    }
}

mod if_else {
    use super::*;

    #[test]
    fn if_true_else_runs_then() {
        assert_parity("if true; then echo yes; else echo no; fi");
    }

    #[test]
    fn if_false_else_runs_else() {
        assert_parity("if false; then echo yes; else echo no; fi");
    }

    #[test]
    fn if_else_with_test() {
        assert_parity(r#"if [[ "$USER" == "" ]]; then echo empty; else echo set; fi"#);
    }
}

mod if_elif {
    use super::*;

    #[test]
    fn if_elif_runs_first_match() {
        assert_parity(r#"if false; then echo n; elif true; then echo got; fi"#);
    }

    #[test]
    fn if_elif_else_all_chain() {
        assert_parity(
            r#"
X=2
if (( X == 1 )); then
  echo one
elif (( X == 2 )); then
  echo two
elif (( X == 3 )); then
  echo three
else
  echo other
fi
"#,
        );
    }

    #[test]
    fn if_elif_else_falls_to_else() {
        assert_parity(
            r#"
X=99
if (( X == 1 )); then
  echo one
elif (( X == 2 )); then
  echo two
else
  echo other
fi
"#,
        );
    }

    #[test]
    fn multiple_elif_no_match() {
        assert_parity(
            r#"
if false; then echo a
elif false; then echo b
elif false; then echo c
fi
echo done
"#,
        );
    }

    /// First matching elif wins; later elifs don't fire.
    #[test]
    fn first_matching_elif_wins() {
        assert_parity(
            r#"
X=2
if (( X == 1 )); then
  echo a
elif (( X >= 2 )); then
  echo b
elif (( X >= 0 )); then
  echo c
fi
"#,
        );
    }
}

mod exit_status {
    use super::*;

    /// if-then exit = then-branch's exit when condition succeeds.
    #[test]
    fn exit_from_then_branch() {
        assert_parity("if true; then false; fi; echo $?");
    }

    /// if exit = else-branch's exit when condition fails.
    #[test]
    fn exit_from_else_branch() {
        assert_parity("if false; then true; else false; fi; echo $?");
    }

    /// No matching branch (no else) → exit 0.
    #[test]
    fn no_else_no_match_exit_zero() {
        assert_parity("if false; then echo y; fi; echo $?");
    }

    /// elif-branch's exit propagates.
    #[test]
    fn exit_from_elif_branch() {
        assert_parity("if false; then true; elif true; then false; fi; echo $?");
    }
}

mod nested {
    use super::*;

    #[test]
    fn if_inside_then() {
        assert_parity(
            r#"
if true; then
  if true; then
    echo inner
  fi
fi
"#,
        );
    }

    #[test]
    fn if_inside_else() {
        assert_parity(
            r#"
if false; then
  echo top-true
else
  if true; then
    echo inner-true
  else
    echo inner-false
  fi
fi
"#,
        );
    }

    #[test]
    fn three_levels_deep() {
        assert_parity(
            r#"
if true; then
  if true; then
    if true; then
      echo deep
    fi
  fi
fi
"#,
        );
    }
}

mod pipeline_condition {
    use super::*;

    /// Condition is a pipeline — uses last stage's exit.
    #[test]
    fn pipeline_as_condition() {
        assert_parity(r#"if echo hi | grep hi >/dev/null; then echo found; fi"#);
    }

    #[test]
    fn negated_pipeline_as_condition() {
        assert_parity(r#"if ! echo hi | grep nope >/dev/null; then echo not-found; fi"#);
    }
}

mod compound_conditions {
    use super::*;

    #[test]
    fn and_condition() {
        assert_parity(r#"if true && true; then echo both; fi"#);
    }

    #[test]
    fn or_condition() {
        assert_parity(r#"if false || true; then echo one; fi"#);
    }

    #[test]
    fn negated_simple() {
        assert_parity(r#"if ! false; then echo not-false; fi"#);
    }

    #[test]
    fn double_negation() {
        assert_parity(r#"if ! ! true; then echo true; fi"#);
    }
}

mod assignment_in_condition {
    use super::*;

    /// Variable assignment as condition — assignment succeeds (exit 0).
    #[test]
    fn assignment_succeeds_as_condition() {
        assert_parity(r#"if X=value; then echo set; fi; echo $X"#);
    }

    /// Sequence of cmds in condition — uses last cmd's exit.
    #[test]
    fn semicolon_chain_uses_last_exit() {
        assert_parity(r#"if true; false; then echo y; else echo n; fi"#);
    }
}

mod multi_line {
    use super::*;

    /// Newline-separated then/else clauses.
    #[test]
    fn multi_line_if_else() {
        assert_parity(
            r#"
if true
then
  echo yes
else
  echo no
fi
"#,
        );
    }
}

/// zsh's alternate `if { cond } { body }` brace-block form, with the
/// `elif { cond } { body }` continuation. The regression that motivated these:
/// a brace-form `elif` (no trailing `else`) followed by a separator and another
/// command aborted the whole parse — the elif loop's skip_separators() ate the
/// separator the outer list needed, making the next command a "STRING after
/// compound" error. That blocked real zinit code (`zinit-install.zsh`) entirely.
mod brace_form {
    use super::*;

    #[test]
    fn brace_if_elif_then_trailing_command() {
        // The exact failing shape: brace-elif, no else, then another command.
        assert_parity("if { true } { print a } elif { false } { print b }\nprint done");
    }

    #[test]
    fn brace_test_cond_elif_brace_trailing_command() {
        // `[[ … ]]` cond + brace body + brace-elif + trailing command.
        assert_parity("if [[ -z x ]] { print a } elif { false } { print b }\nprint done");
    }

    #[test]
    fn brace_if_elif_taken_then_trailing_command() {
        // The elif branch fires, and the trailing command still runs.
        assert_parity("if { false } { print a } elif { true } { print b }\nprint done");
    }

    #[test]
    fn brace_if_elif_else_chain() {
        assert_parity("if { false } { print a } elif { false } { print b } else { print c }");
    }
}
