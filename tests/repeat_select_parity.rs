//! `repeat N { ... }` and `select word in list; do ... done` parity.
//!
//! `repeat` is a zsh-specific count-loop; `select` is a Korn-shell-style
//! menu loop (interactive — we drive it via stdin).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
#[allow(dead_code)]
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

fn run_with_stdin(bin: &Path, script: &str, stdin: &str) -> R {
    let mut child = Command::new(bin)
        .args(if bin.file_name().map(|n| n == "zsh").unwrap_or(false) {
            vec!["-fc", script]
        } else {
            vec!["--zsh", "-f", "-c", script]
        })
        .env_remove("ZSHRS_CACHE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .expect("write");
    let o = child.wait_with_output().expect("wait");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
/// Only compare stdout + exit. Stderr is excluded because select's prompt
/// (PS3) goes to stderr and can pick up inherited environment formatting
/// that varies between test runs.
fn assert_parity_with_stdin(script: &str, stdin: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_with_stdin(Path::new(zsh_path()), script, stdin);
    let r = run_with_stdin(&zshrs_bin(), script, stdin);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{script}\nstdin: {stdin:?}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod repeat_basic {
    use super::*;

    #[test]
    fn repeat_3_times_prints() {
        assert_parity(r#"repeat 3 echo hi"#);
    }

    #[test]
    fn repeat_with_braces() {
        assert_parity(r#"repeat 3 { echo hi }"#);
    }

    #[test]
    fn repeat_0_no_iterations() {
        assert_parity(r#"repeat 0 echo hi; echo done"#);
    }

    #[test]
    fn repeat_5_count_iterations() {
        assert_parity(r#"n=0; repeat 5 (( n++ )); echo $n"#);
    }
}

mod repeat_arith {
    use super::*;

    /// repeat takes an arith expression.
    #[test]
    fn repeat_with_arith_expr() {
        assert_parity(r#"repeat $((2+1)) echo hi"#);
    }

    /// repeat with variable count.
    #[test]
    fn repeat_with_var_count() {
        assert_parity(r#"n=4; repeat $n echo hi"#);
    }
}

mod repeat_with_loop_body {
    use super::*;

    /// repeat body can contain newlines.
    #[test]
    fn repeat_multiline_body() {
        assert_parity(
            r#"
repeat 2 do
  echo step
  echo done-step
done
"#,
        );
    }

    /// repeat body with conditional.
    #[test]
    fn repeat_with_conditional_inside() {
        assert_parity(r#"i=0; repeat 5 { (( i++ )); if (( i % 2 == 0 )); then echo even-$i; fi }"#);
    }
}

mod repeat_break_continue {
    use super::*;

    /// break exits repeat early.
    #[test]
    fn repeat_break() {
        assert_parity(
            r#"
i=0
repeat 10 do
  (( i++ ))
  if (( i == 3 )); then break; fi
  echo $i
done
echo final=$i
"#,
        );
    }

    /// continue skips to next iteration.
    #[test]
    fn repeat_continue() {
        assert_parity(
            r#"
i=0
repeat 5 do
  (( i++ ))
  if (( i == 3 )); then continue; fi
  echo $i
done
"#,
        );
    }
}

mod select_basic {
    use super::*;

    /// select prints menu on stderr, reads choice from stdin.
    /// User selects "1" → first item.
    #[test]
    fn select_choose_first() {
        assert_parity_with_stdin(
            r#"select x in apple banana cherry; do echo "chose:$x"; break; done"#,
            "1\n",
        );
    }

    #[test]
    fn select_choose_second() {
        assert_parity_with_stdin(
            r#"select x in apple banana cherry; do echo "chose:$x"; break; done"#,
            "2\n",
        );
    }

    /// Invalid number → x is empty, REPLY set.
    #[test]
    fn select_invalid_choice_empty_value() {
        assert_parity_with_stdin(
            r#"select x in apple banana; do echo "x=[$x] REPLY=[$REPLY]"; break; done"#,
            "99\n",
        );
    }

    /// EOF on stdin → select loop exits.
    #[test]
    fn select_eof_exits_loop() {
        assert_parity_with_stdin(r#"select x in a b c; do echo $x; done; echo after"#, "");
    }
}

mod select_in_positional {
    use super::*;

    /// `select x` (no in-list) → iterates over positional params.
    #[test]
    fn select_positional_no_in_list() {
        assert_parity_with_stdin(
            r#"set -- alpha beta gamma; select x do echo "chose:$x"; break; done"#,
            "2\n",
        );
    }
}

mod select_prompt {
    use super::*;

    /// PS3 controls the prompt.
    #[test]
    fn select_uses_ps3_prompt() {
        assert_parity_with_stdin(
            r#"PS3='Pick> '; select x in a b; do echo $x; break; done"#,
            "1\n",
        );
    }
}

mod select_blank_input {
    use super::*;

    /// Blank input → repeats prompt.
    #[test]
    fn select_blank_then_choice() {
        assert_parity_with_stdin(r#"select x in a b; do echo "got:$x"; break; done"#, "\n1\n");
    }
}
