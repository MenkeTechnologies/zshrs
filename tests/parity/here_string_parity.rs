//! Here-string `<<<` parity tests.

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
    fn cat_here_string_literal() {
        assert_parity(r#"cat <<< "hello""#);
    }

    #[test]
    fn cat_here_string_unquoted_word() {
        assert_parity(r#"cat <<< hello"#);
    }

    #[test]
    fn cat_here_string_empty() {
        assert_parity(r#"cat <<< """#);
    }

    /// `<<<` appends a newline.
    #[test]
    fn here_string_adds_trailing_newline() {
        assert_parity(r#"cat <<< "x" | wc -c"#);
    }
}

mod variable_expansion {
    use super::*;

    #[test]
    fn here_string_expands_var() {
        assert_parity(r#"X=hello; cat <<< "$X""#);
    }

    #[test]
    fn here_string_expands_arithmetic() {
        assert_parity(r#"cat <<< "$((1+2))""#);
    }

    #[test]
    fn here_string_expands_command_subst() {
        assert_parity(r#"cat <<< "$(echo nested)""#);
    }

    #[test]
    fn here_string_with_multiple_vars() {
        assert_parity(r#"A=hi; B=world; cat <<< "$A $B""#);
    }
}

mod with_read {
    use super::*;

    #[test]
    fn read_var_from_here_string() {
        assert_parity(r#"read x <<< "value"; echo "[$x]""#);
    }

    #[test]
    fn read_multiple_vars() {
        assert_parity(r#"read a b c <<< "one two three"; echo "$a/$b/$c""#);
    }

    /// `read -A` reads into array.
    #[test]
    fn read_array_from_here_string() {
        assert_parity(r#"read -A arr <<< "a b c"; echo "${#arr} ${arr[1]}""#);
    }
}

mod with_grep {
    use super::*;

    #[test]
    fn grep_here_string_matches() {
        assert_parity(r#"grep foo <<< "foobar" 2>/dev/null; echo exit=$?"#);
    }

    #[test]
    fn grep_here_string_no_match() {
        assert_parity(r#"grep xyz <<< "foobar" 2>/dev/null; echo exit=$?"#);
    }
}

mod nested_quoting {
    use super::*;

    /// Single-quoted here-string is literal.
    #[test]
    fn single_quoted_here_string_literal() {
        assert_parity(r#"X=expanded; cat <<< '$X'"#);
    }

    /// Double-quoted with escaped dollar still literal.
    #[test]
    fn escaped_dollar_in_here_string() {
        assert_parity(r#"X=expanded; cat <<< "\$X""#);
    }
}

mod multiline_value {
    use super::*;

    /// `<<<` with embedded newlines via $'...'.
    #[test]
    fn ansi_c_quoted_multiline() {
        assert_parity(r#"cat <<< $'line1\nline2\nline3'"#);
    }
}

mod special_chars {
    use super::*;

    #[test]
    fn here_string_with_glob_chars_unexpanded() {
        // Glob chars literal inside here-string (no filename expansion).
        assert_parity(r#"cat <<< "*.txt""#);
    }

    #[test]
    fn here_string_with_tab() {
        assert_parity(r#"cat <<< $'a\tb' | cat -t"#);
    }
}

mod combine_with_redirects {
    use super::*;

    /// `<<<` combined with explicit stdin descriptor.
    #[test]
    fn here_string_to_fd_0() {
        assert_parity(r#"cat 0<<< "abc""#);
    }

    /// Pipe to next command after here-string.
    #[test]
    fn here_string_piped_to_tr() {
        assert_parity(r#"tr a-z A-Z <<< "hello""#);
    }
}

/// c:Src/builtin.c:6855 / c:7039 — `read` separates fields with `isep(c)` /
/// `iwsep(c)` from the typtab built out of $IFS. With MULTIBYTE unset an 8-bit
/// IFS byte is one separator byte. zshrs holds such a byte as a two-char Meta
/// pair in both $IFS and the line, and `read` tested the chars one by one, so
/// it split at the Meta char and kept the other half (`p` / `Éq`).
mod read_eight_bit_ifs_without_multibyte {
    use super::*;

    #[test]
    fn eight_bit_ifs_byte_separates_fields() {
        assert_parity(r#"unsetopt multibyte; IFS=$'\xe9'; read -r x y <<< $'p\xe9q'; print -r -- "[$x][$y]""#);
        assert_parity(r#"unsetopt multibyte; IFS=$'\xc3\xa9'; read -r x y z <<< $'foo\xc3bar\xa9boo'; print -r -- "[$x][$y][$z]""#);
        assert_parity(r#"unsetopt multibyte; IFS=$'\xe9'; read -rA a <<< $'p\xe9q\xe9\xe9r'; print -r -- ${#a} "[${(j:][:)a}]""#);
        assert_parity(r#"unsetopt multibyte; IFS=$'\xe9'; read -r x <<< $'p\xe9q'; print -rn -- $x | od -An -tx1"#);
    }

    #[test]
    fn ascii_and_multibyte_ifs_are_unchanged() {
        assert_parity(r#"IFS=é; read -r x y <<< "pé q"; print -r -- "[$x][$y]"; IFS=: ; read -rA a <<< "a:b::c"; print -r -- ${#a} "${a[@]}""#);
        assert_parity(r#"IFS=' :'; read -r x y z <<< "  a : b  c  "; print -r -- "[$x][$y][$z]"; read -r w <<< "  lone  "; print -r -- "[$w]""#);
    }
}
