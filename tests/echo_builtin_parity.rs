//! `echo` builtin parity: -n, -e, -E, backslash escapes, --
//!
//! NB: zsh defaults to `setopt BSD_ECHO` off, so `echo` processes
//! escapes by default (different from bash without -e).

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
    fn echo_simple() {
        assert_parity(r#"echo hello"#);
    }

    #[test]
    fn echo_empty() {
        assert_parity(r#"echo"#);
    }

    #[test]
    fn echo_multiple_args_joined_by_space() {
        assert_parity(r#"echo a b c"#);
    }

    #[test]
    fn echo_multiple_args_with_extra_whitespace() {
        // Whitespace-collapsed to single space by word-splitting,
        // then re-joined by single space.
        assert_parity(r#"echo   a    b"#);
    }
}

mod minus_n {
    use super::*;

    #[test]
    fn echo_dash_n_no_newline() {
        assert_parity(r#"echo -n "no newline"; echo END"#);
    }

    #[test]
    fn echo_dash_n_empty() {
        assert_parity(r#"echo -n ""; echo END"#);
    }

    #[test]
    fn echo_dash_n_with_args() {
        assert_parity(r#"echo -n a b c; echo END"#);
    }
}

mod escapes_default_on {
    use super::*;

    /// zsh's echo processes \n by default (unlike bash without -e).
    #[test]
    fn echo_processes_newline_escape_by_default() {
        assert_parity(r#"echo 'a\nb'"#);
    }

    #[test]
    fn echo_processes_tab_escape_by_default() {
        assert_parity(r#"echo 'a\tb' | cat -t"#);
    }

    #[test]
    fn echo_processes_backslash_by_default() {
        assert_parity(r#"echo 'a\\b'"#);
    }

    /// `\c` truncates (no newline + stop processing).
    #[test]
    fn echo_backslash_c_truncates() {
        assert_parity(r#"echo 'a\cb'; echo END"#);
    }

    /// Octal \NNN.
    #[test]
    fn echo_octal_escape() {
        assert_parity(r#"echo '\101' | cat -v"#);
    }

    /// Hex \xNN.
    #[test]
    fn echo_hex_escape() {
        assert_parity(r#"echo '\x41' | cat -v"#);
    }

    #[test]
    fn echo_carriage_return_escape() {
        assert_parity(r#"echo 'a\rb' | cat -v"#);
    }

    #[test]
    fn echo_form_feed_escape() {
        assert_parity(r#"echo 'a\fb' | cat -v"#);
    }

    #[test]
    fn echo_backspace_escape() {
        assert_parity(r#"echo 'a\bb' | cat -v"#);
    }

    #[test]
    fn echo_alert_escape() {
        assert_parity(r#"echo 'x\ay' | cat -v"#);
    }
}

mod minus_E {
    use super::*;

    /// `echo -E` disables escape processing.
    #[test]
    fn echo_dash_E_keeps_backslash_literal() {
        assert_parity(r#"echo -E 'a\nb'"#);
    }

    #[test]
    fn echo_dash_E_with_tab_escape_literal() {
        assert_parity(r#"echo -E 'a\tb'"#);
    }
}

mod minus_e {
    use super::*;

    /// `echo -e` explicitly enables escapes (zsh's default behavior).
    #[test]
    fn echo_dash_e_processes_newline() {
        assert_parity(r#"echo -e 'a\nb'"#);
    }

    #[test]
    fn echo_dash_e_processes_tab() {
        assert_parity(r#"echo -e 'a\tb' | cat -t"#);
    }
}

mod no_special_dash_handling {
    use super::*;

    /// echo --: zsh treats `--` as a literal argument (echo doesn't
    /// implement option-terminator).
    #[test]
    fn echo_double_dash_is_literal() {
        assert_parity(r#"echo -- abc"#);
    }

    /// echo -X (unknown flag) — zsh treats unknown flags as literal.
    #[test]
    fn echo_unknown_flag_is_literal() {
        assert_parity(r#"echo -X abc"#);
    }
}

mod combined_flags {
    use super::*;

    /// `echo -ne` combines both flags.
    #[test]
    fn echo_dash_ne_combined() {
        assert_parity(r#"echo -ne 'a\nb'; echo END"#);
    }

    #[test]
    fn echo_dash_En_combined() {
        assert_parity(r#"echo -En 'a\nb'; echo END"#);
    }
}

mod with_variables {
    use super::*;

    #[test]
    fn echo_var() {
        assert_parity(r#"X=hi; echo "$X""#);
    }

    /// Backslashes inside a variable expansion are not re-processed
    /// (because `-e` only acts on the literal echo args).
    #[test]
    fn echo_var_with_backslash_n() {
        assert_parity(r#"X='a\nb'; echo "$X""#);
    }
}

mod special_zsh_options {
    use super::*;

    /// `setopt bsd_echo` makes echo NOT interpret escapes (BSD style).
    #[test]
    fn bsd_echo_disables_escapes() {
        assert_parity(r#"setopt bsd_echo; echo 'a\nb'"#);
    }

    /// `setopt no_bsd_echo` (default) — echo processes escapes.
    #[test]
    fn no_bsd_echo_processes_escapes() {
        assert_parity(r#"unsetopt bsd_echo; echo 'a\nb'"#);
    }
}
