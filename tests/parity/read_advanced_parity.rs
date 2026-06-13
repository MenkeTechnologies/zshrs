//! `read` builtin advanced-flag parity:
//! -r, -d delim, -n N, -A array, -k chars, -t timeout, prompt, REPLY.

#![allow(non_snake_case)]

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
struct R {
    stdout: String,
    exit: i32,
}

fn run_with_stdin(bin: &Path, script: &str, stdin_bytes: &[u8]) -> R {
    let args: Vec<&str> = if bin.file_name().map(|n| n == "zsh").unwrap_or(false) {
        vec!["-fc", script]
    } else {
        vec!["--zsh", "-f", "-c", script]
    };
    let mut child = Command::new(bin)
        .args(args)
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
        .write_all(stdin_bytes)
        .expect("write");
    let o = child.wait_with_output().expect("wait");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn assert_parity_stdin(script: &str, stdin_bytes: &[u8]) {
    if !zsh_available() {
        return;
    }
    let z = run_with_stdin(Path::new(zsh_path()), script, stdin_bytes);
    let r = run_with_stdin(&zshrs_bin(), script, stdin_bytes);
    assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{script}\nstdin: {stdin_bytes:?}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}", z.stdout, r.stdout);
    assert_eq!(z.exit, r.exit);
}

mod basic {
    use super::*;

    #[test]
    fn read_single_var() {
        assert_parity_stdin(r#"read x; echo "[$x]""#, b"hello\n");
    }

    #[test]
    fn read_multiple_vars_word_split() {
        assert_parity_stdin(r#"read a b c; echo "$a|$b|$c""#, b"one two three\n");
    }

    /// More vars than words → trailing vars empty.
    #[test]
    fn read_more_vars_than_words() {
        assert_parity_stdin(r#"read a b c; echo "[$a][$b][$c]""#, b"only\n");
    }

    /// More words than vars → last var gets rest.
    #[test]
    fn read_more_words_than_vars_rest_in_last() {
        assert_parity_stdin(r#"read a b; echo "[$a][$b]""#, b"one two three four\n");
    }

    /// No vars → REPLY set.
    #[test]
    fn read_no_var_uses_reply() {
        assert_parity_stdin(r#"read; echo "[$REPLY]""#, b"value\n");
    }
}

mod dash_r {
    use super::*;

    /// Without -r, backslash is escape.
    #[test]
    fn no_dash_r_backslash_escape() {
        assert_parity_stdin(r#"read x; echo "[$x]""#, b"a\\nb\n");
    }

    /// With -r, backslash is literal.
    #[test]
    fn dash_r_backslash_literal() {
        assert_parity_stdin(r#"read -r x; echo "[$x]""#, b"a\\nb\n");
    }

    /// -r prevents line continuation.
    #[test]
    fn dash_r_no_line_continuation() {
        assert_parity_stdin(r#"read -r x; echo "[$x]""#, b"abc\\\n");
    }
}

mod dash_d_delim {
    use super::*;

    /// `read -d :` reads until `:` (not newline).
    #[test]
    fn dash_d_custom_delim() {
        assert_parity_stdin(r#"read -d : x; echo "[$x]""#, b"hello:world\n");
    }

    /// `read -d ''` reads until NUL.
    #[test]
    fn dash_d_nul_delim() {
        assert_parity_stdin(
            r#"read -d '' x; echo "[$x]""#,
            b"all of this\nincluding newlines\0",
        );
    }
}

mod dash_n {
    use super::*;

    /// `read -n N var` reads up to N bytes.
    #[test]
    fn dash_n_read_n_chars() {
        assert_parity_stdin(r#"read -k 5 x; echo "[$x]""#, b"abcdefg\n");
    }
}

mod dash_A_array {
    use super::*;

    /// `read -A arr` reads line into array (word-split).
    #[test]
    fn dash_A_into_array() {
        assert_parity_stdin(
            r#"read -A arr; echo "${#arr}|${arr[1]}|${arr[2]}|${arr[3]}""#,
            b"one two three\n",
        );
    }

    /// `read -A` with single word.
    #[test]
    fn dash_A_single_word() {
        assert_parity_stdin(r#"read -A arr; echo "${#arr}|${arr[1]}""#, b"hello\n");
    }
}

mod multi_line {
    use super::*;

    /// One `read` per line.
    #[test]
    fn read_one_line_at_a_time() {
        assert_parity_stdin(
            r#"
read a
read b
read c
echo "$a|$b|$c"
"#,
            b"first\nsecond\nthird\n",
        );
    }

    /// Loop `while read line` over entire stdin.
    #[test]
    fn while_read_loop_over_stdin() {
        assert_parity_stdin(
            r#"
while read line; do
  echo "got: $line"
done
"#,
            b"alpha\nbeta\ngamma\n",
        );
    }
}

mod ifs_handling {
    use super::*;

    /// Custom IFS affects word-splitting in read.
    #[test]
    fn ifs_colon_word_split() {
        assert_parity_stdin(
            r#"IFS=: read a b c; echo "[$a][$b][$c]""#,
            b"alpha:beta:gamma\n",
        );
    }

    /// Multi-char IFS.
    #[test]
    fn ifs_multi_char_split() {
        assert_parity_stdin(r#"IFS=:, read a b c; echo "[$a][$b][$c]""#, b"x:y,z\n");
    }

    /// Empty IFS — no splitting.
    #[test]
    fn ifs_empty_no_split() {
        assert_parity_stdin(r#"IFS= read x; echo "[$x]""#, b"  spaces preserved  \n");
    }
}

mod prompt_with_dash_p {
    use super::*;

    /// `read -p PROMPT var` — zsh's -p means coproc-fd, not bash's prompt.
    /// `read "?prompt" var` is zsh's prompt syntax.
    #[test]
    fn zsh_prompt_via_question_mark() {
        assert_parity_stdin(r#"read "?Name: " x 2>/dev/null; echo "[$x]""#, b"jacob\n");
    }

    /// `read var?prompt` — c:Src/builtin.c:6534-6543 splits the FIRST
    /// arg at its first `?`: name before, prompt after (printed only
    /// when interactive; suppressed here since stdin is a pipe).
    #[test]
    fn zsh_prompt_embedded_in_first_name() {
        assert_parity_stdin(r#"read "v?myprompt: "; echo "[$v]""#, b"hi\n");
    }

    /// Only the first name carries the `?prompt`; later names are
    /// plain variable names (c:6445 firstarg aliases args[0] only).
    #[test]
    fn zsh_prompt_split_with_second_var() {
        assert_parity_stdin(r#"read "x?p: " y; echo "[$x][$y]""#, b"a b\n");
    }

    /// Leading-`?` arg is prompt-only — reply defaults to REPLY
    /// (c:6445-6446 `*args++` consumes it before the reply pick).
    #[test]
    fn zsh_prompt_only_arg_defaults_reply() {
        assert_parity_stdin(r#"read "?just prompt"; echo "[$REPLY]""#, b"hi\n");
    }
}

mod eof_handling {
    use super::*;

    /// EOF on stdin → read returns nonzero.
    #[test]
    fn read_eof_nonzero() {
        assert_parity_stdin(r#"read x; echo "exit=$? value=[$x]""#, b"");
    }

    /// Partial line + EOF (no trailing newline).
    #[test]
    fn read_partial_line_eof() {
        assert_parity_stdin(r#"read x; echo "exit=$? value=[$x]""#, b"noeolEOF");
    }
}

mod zero_byte_data {
    use super::*;

    /// Read line containing NUL byte (handling varies).
    #[test]
    fn read_line_with_nul() {
        assert_parity_stdin(r#"read x; echo "[$x]" | od -c | head -1"#, b"abc\0def\n");
    }
}

mod escape_processing {
    use super::*;

    /// Without -r, `\n` becomes literal n (continuation).
    #[test]
    fn no_r_backslash_n_becomes_n() {
        assert_parity_stdin(r#"read x; echo "[$x]""#, b"foo\\bar\n");
    }
}
