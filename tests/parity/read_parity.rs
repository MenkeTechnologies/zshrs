//! `read` builtin parity tests.

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

mod basic {
    use super::*;

    #[test]
    fn read_single_var_from_heredoc() {
        assert_parity("read X <<< hello; echo $X");
    }

    #[test]
    fn read_two_vars_from_heredoc() {
        assert_parity("read X Y <<< 'one two'; echo \"$X|$Y\"");
    }

    /// With more args than vars, last var gets remaining tokens (space-joined).
    #[test]
    fn read_three_vars_last_captures_rest() {
        assert_parity("read X Y Z <<< 'a b c d e'; echo \"[$X][$Y][$Z]\"");
    }

    #[test]
    fn read_more_vars_than_input_empties_extras() {
        assert_parity("read X Y Z <<< 'only'; echo \"[$X][$Y][$Z]\"");
    }

    #[test]
    fn read_no_input_eof_exit_nonzero() {
        assert_parity("read X </dev/null; echo $?");
    }

    /// `read` with no var defaults to $REPLY.
    #[test]
    fn read_no_var_uses_reply() {
        assert_parity("read <<< hello; echo $REPLY");
    }
}

mod raw_mode {
    use super::*;

    /// Without -r, backslash escapes are processed (line continuation).
    #[test]
    fn read_default_consumes_backslash() {
        assert_parity(r#"read X <<< 'a\ b'; echo "[$X]""#);
    }

    /// `-r` raw mode preserves backslashes.
    #[test]
    fn read_r_preserves_backslash() {
        assert_parity(r#"read -r X <<< 'a\ b'; echo "[$X]""#);
    }
}

mod delimiter {
    use super::*;

    /// `-d X` uses X as delimiter instead of newline.
    #[test]
    fn read_d_custom_delimiter_colon() {
        assert_parity(r#"read -d : X <<< 'before:after'; echo "[$X]""#);
    }

    /// `-d ''` reads ALL input (no delimiter).
    #[test]
    fn read_d_empty_consumes_all() {
        assert_parity(r#"read -d '' X <<< 'a b c'; echo "[$X]""#);
    }
}

mod count_chars {
    use super::*;

    /// `-k N` reads N characters (zsh-specific).
    #[test]
    fn read_k_reads_n_chars() {
        assert_parity("read -k 3 X <<< 'abcdef'; echo $X");
    }
}

/// `read -q` reads ONE char: "yes" (status 0, REPLY=y) iff it is exactly
/// 'y'/'Y', else "no" (status 1, REPLY=n) — c:Src/builtin.c:6730-6742.
mod yes_no {
    use super::*;

    #[test]
    fn read_q_yes_lowercase() {
        assert_parity("read -q -u0 <<<y; echo \"$?:$REPLY\"");
    }

    #[test]
    fn read_q_yes_uppercase() {
        assert_parity("read -q -u0 <<<Y; echo \"$?:$REPLY\"");
    }

    #[test]
    fn read_q_no_n() {
        assert_parity("read -q -u0 <<<n; echo \"$?:$REPLY\"");
    }

    #[test]
    fn read_q_other_is_no() {
        assert_parity("read -q -u0 <<<X; echo \"$?:$REPLY\"");
    }

    /// Loop over y/Y/n/N/X — the workers/42248-style status sequence.
    #[test]
    fn read_q_sequence() {
        assert_parity("for c in y Y n N X; do read -q -u0 <<<$c; print $?; done");
    }
}

mod from_pipeline {
    use super::*;

    /// `while read line; do …` loop pattern.
    #[test]
    fn while_read_loop_processes_each_line() {
        assert_parity(
            r#"
while read line; do
  echo "got: $line"
done <<EOF
one
two
three
EOF
"#,
        );
    }

    #[test]
    fn while_read_from_pipe() {
        assert_parity(
            r#"
echo -e "a\nb\nc" | while read line; do
  echo "X $line"
done
"#,
        );
    }

    #[test]
    fn while_read_handles_lines_with_spaces() {
        assert_parity(
            r#"
while read line; do
  echo "[$line]"
done <<EOF
hello world
two   spaces
EOF
"#,
        );
    }
}

mod array_read {
    use super::*;

    /// `read -A arr` reads input into array, splitting on $IFS.
    #[test]
    fn read_A_into_array() {
        assert_parity(r#"read -A arr <<< 'a b c d'; print -l "${arr[@]}""#);
    }
}

mod with_prompt {
    use super::*;

    /// `read -s` silent mode — doesn't echo input. Use with stdin
    /// redirect so we don't need a terminal.
    #[test]
    fn read_s_silent_still_reads_value() {
        assert_parity("read -s X <<< secret; echo $X");
    }
}

mod ifs_handling {
    use super::*;

    /// Custom IFS splits on that char.
    #[test]
    fn ifs_colon_splits_on_colon() {
        assert_parity(r#"IFS=: read X Y Z <<< 'a:b:c'; echo "[$X][$Y][$Z]""#);
    }

    /// Empty IFS — no splitting, whole line goes to first var.
    #[test]
    fn ifs_empty_no_splitting() {
        assert_parity(r#"IFS= read X Y <<< 'one two three'; echo "[$X][$Y]""#);
    }
}
