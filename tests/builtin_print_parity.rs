//! print / echo / printf parity tests against real zsh 5.9.

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

mod print_basic {
    use super::*;

    #[test]
    fn print_single_word() {
        assert_parity("print hello");
    }

    #[test]
    fn print_multi_words_space_separated() {
        assert_parity("print a b c");
    }

    #[test]
    fn print_no_args_emits_newline() {
        assert_parity("print");
    }

    #[test]
    fn print_n_suppresses_newline() {
        assert_parity("print -n hello; print done");
    }

    #[test]
    fn print_l_one_per_line() {
        assert_parity("print -l a b c");
    }

    #[test]
    fn print_r_raw_no_escapes() {
        assert_parity(r#"print -r '\n'"#);
    }

    #[test]
    fn print_rn_raw_no_newline() {
        assert_parity(r#"print -rn 'hello'; print done"#);
    }
}

mod print_escapes {
    use super::*;

    /// print processes \n \t \r \\ by default.
    #[test]
    fn print_processes_escape_sequences() {
        assert_parity(r#"print 'a\nb\tc'"#);
    }

    /// print -r disables escape processing.
    #[test]
    fn print_r_disables_escapes() {
        assert_parity(r#"print -r 'a\nb\tc'"#);
    }

    /// print processes \xNN hex escape.
    #[test]
    fn print_processes_hex_escape() {
        assert_parity(r#"print '\x41'"#);
    }

    /// print -- terminates option parsing.
    #[test]
    fn print_dash_dash_terminates_options() {
        assert_parity(r#"print -- "-n"; print done"#);
    }
}

mod echo_basic {
    use super::*;

    #[test]
    fn echo_single_word() {
        assert_parity("echo hello");
    }

    #[test]
    fn echo_multi_words() {
        assert_parity("echo a b c");
    }

    #[test]
    fn echo_no_args() {
        assert_parity("echo");
    }

    #[test]
    fn echo_n_suppresses_newline() {
        assert_parity("echo -n hello; echo done");
    }

    /// zsh's echo processes escapes by default (different from bash).
    #[test]
    fn echo_processes_escape_n() {
        assert_parity(r#"echo 'a\nb'"#);
    }

    #[test]
    #[allow(non_snake_case)]
    fn echo_E_disables_escapes() {
        assert_parity(r#"echo -E 'a\nb'"#);
    }
}

mod printf_basic {
    use super::*;

    #[test]
    fn printf_string_format() {
        assert_parity(r#"printf "%s\n" hello"#);
    }

    #[test]
    fn printf_no_args_uses_empty_string() {
        assert_parity(r#"printf "%s\n""#);
    }

    #[test]
    fn printf_multi_args_loops_format() {
        // printf re-uses format string for remaining args
        assert_parity(r#"printf "%s\n" a b c"#);
    }

    #[test]
    fn printf_integer_format() {
        assert_parity(r#"printf "%d\n" 42"#);
    }

    #[test]
    fn printf_negative_integer() {
        assert_parity(r#"printf "%d\n" -7"#);
    }

    #[test]
    fn printf_hex_format() {
        assert_parity(r#"printf "%x\n" 255"#);
    }

    #[test]
    fn printf_octal_format() {
        assert_parity(r#"printf "%o\n" 15"#);
    }

    #[test]
    #[allow(non_snake_case)]
    fn printf_capital_X_uppercase_hex() {
        assert_parity(r#"printf "%X\n" 255"#);
    }

    #[test]
    fn printf_percent_d_with_zero_padding() {
        assert_parity(r#"printf "%05d\n" 42"#);
    }

    #[test]
    fn printf_percent_s_with_width() {
        assert_parity(r#"printf "[%10s]\n" hi"#);
    }

    #[test]
    fn printf_percent_minus_s_left_align() {
        assert_parity(r#"printf "[%-10s]\n" hi"#);
    }

    #[test]
    fn printf_literal_percent_via_double_percent() {
        assert_parity(r#"printf "100%%\n""#);
    }

    #[test]
    fn printf_no_newline_in_format() {
        assert_parity(r#"printf "abc"; printf done"#);
    }

    #[test]
    fn printf_escape_sequence_n() {
        assert_parity(r#"printf "a\nb\n""#);
    }

    #[test]
    fn printf_escape_sequence_t() {
        assert_parity(r#"printf "a\tb\n""#);
    }
}

mod print_with_flags {
    use super::*;

    /// print -P processes prompt-escape sequences. `%%` → `%`.
    #[test]
    #[allow(non_snake_case)]
    fn print_P_double_percent_yields_single() {
        assert_parity(r#"print -P '%%'"#);
    }

    /// print -P with plain text — passes through.
    #[test]
    #[allow(non_snake_case)]
    fn print_P_plain_text_unchanged() {
        assert_parity(r#"print -P 'hello'"#);
    }

    /// `-D` print directory style — pin behaviour exists.
    #[test]
    fn print_dash_l_with_multiple_args() {
        assert_parity("print -l one two three four");
    }
}

mod print_array {
    use super::*;

    #[test]
    fn print_array_via_splat() {
        assert_parity(r#"arr=(a b c); print "${arr[@]}""#);
    }

    #[test]
    fn print_l_array_one_per_line() {
        assert_parity(r#"arr=(a b c); print -l "${arr[@]}""#);
    }

    #[test]
    fn print_empty_array_via_splat() {
        assert_parity(r#"arr=(); print -l "${arr[@]}"; print done"#);
    }
}
