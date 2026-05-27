//! Here-doc / here-string parity tests.

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

mod basic_heredoc {
    use super::*;

    #[test]
    fn simple_heredoc_to_cat() {
        assert_parity("cat <<EOF\nhello\nEOF");
    }

    #[test]
    fn heredoc_multiple_lines() {
        assert_parity("cat <<EOF\nline1\nline2\nline3\nEOF");
    }

    #[test]
    fn heredoc_empty_body() {
        assert_parity("cat <<EOF\nEOF");
    }

    #[test]
    fn heredoc_with_blank_line_in_body() {
        assert_parity("cat <<EOF\nfirst\n\nthird\nEOF");
    }

    #[test]
    fn heredoc_with_custom_delim_word() {
        assert_parity("cat <<DELIM\nbody\nDELIM");
    }
}

mod heredoc_var_expansion {
    use super::*;

    /// Unquoted delim → parameter expansion happens inside body.
    #[test]
    fn unquoted_delim_expands_dollar_var() {
        assert_parity(
            r#"X=value; cat <<EOF
hello $X
EOF"#,
        );
    }

    #[test]
    fn unquoted_delim_expands_braced_var() {
        assert_parity(
            r#"X=value; cat <<EOF
hello ${X}
EOF"#,
        );
    }

    /// Single-quoted delim → NO parameter expansion (literal $X).
    #[test]
    fn single_quoted_delim_no_var_expansion() {
        assert_parity(
            r#"X=value; cat <<'EOF'
hello $X
EOF"#,
        );
    }

    /// Double-quoted delim → same as unquoted (expansion happens).
    #[test]
    fn double_quoted_delim_expands_dollar_var() {
        assert_parity(
            r#"X=value; cat <<"EOF"
hello $X
EOF"#,
        );
    }

    /// Backslash-escaped delim → no expansion (per zsh docs).
    #[test]
    fn backslash_escaped_delim_no_expansion() {
        assert_parity(
            r#"X=value; cat <<\EOF
hello $X
EOF"#,
        );
    }
}

mod indented_heredoc {
    use super::*;

    /// `<<-EOF` strips LEADING TABS (not spaces).
    #[test]
    fn dash_form_strips_leading_tabs() {
        assert_parity("cat <<-EOF\n\thello\n\tEOF");
    }

    /// `<<-EOF` does NOT strip leading SPACES — terminator on a
    /// space-indented line isn't matched, both shells hang waiting
    /// for EOF. Disable to keep the test suite responsive; the
    /// behavior is "both shells hang the same way" anyway.
    #[test]
    #[ignore = "BOTH SHELLS HANG: <<-EOF with space-indented EOF terminator never matches"]
    fn dash_form_preserves_leading_spaces() {
        assert_parity("cat <<-EOF\n    hello\n    EOF");
    }

    #[test]
    fn dash_form_mixed_tab_and_text() {
        assert_parity("cat <<-EOF\n\t\thello\n\tworld\n\tEOF");
    }
}

mod heredoc_command_subst {
    use super::*;

    /// Command substitution inside heredoc body (unquoted delim).
    #[test]
    fn cmdsubst_inside_unquoted_heredoc() {
        assert_parity(
            r#"cat <<EOF
result: $(echo "from cmd")
EOF"#,
        );
    }

    /// No cmd subst when delim is single-quoted.
    #[test]
    fn cmdsubst_literal_when_delim_quoted() {
        assert_parity(
            r#"cat <<'EOF'
result: $(echo "from cmd")
EOF"#,
        );
    }
}

mod here_string {
    use super::*;

    #[test]
    fn here_string_simple() {
        assert_parity(r#"cat <<< "hello""#);
    }

    #[test]
    fn here_string_with_variable() {
        assert_parity(r#"X=foo; cat <<< "$X""#);
    }

    #[test]
    fn here_string_no_quotes() {
        assert_parity(r#"cat <<< hello"#);
    }

    /// Each here-string gets a trailing newline added by zsh.
    #[test]
    fn here_string_appends_newline() {
        assert_parity(r#"cat <<< "no_newline" | wc -c"#);
    }
}

mod round_pins {
    use super::*;

    #[test]
    fn heredoc_unquoted_eof() {
        assert_parity("cat <<EOF\nx\nEOF");
    }

    #[test]
    fn heredoc_tab_stripped_marker() {
        assert_parity("cat <<-EOF\n\tx\nEOF");
    }

    #[test]
    fn heredoc_quoted_delim_no_expand() {
        assert_parity("cat <<\\EOF\n$x\nEOF");
    }

    #[test]
    fn here_string_unquoted_word() {
        assert_parity("read -r x <<< word; print -r $x");
    }
}

mod heredoc_pipeline {
    use super::*;

    #[test]
    fn heredoc_piped_to_grep() {
        assert_parity("cat <<EOF | grep hello\nhello world\nfoo bar\nEOF");
    }

    #[test]
    fn heredoc_in_subshell() {
        assert_parity("( cat <<EOF\nfrom subshell\nEOF\n)");
    }
}
