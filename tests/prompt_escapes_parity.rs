//! `print -P` prompt-escape parity tests.
//!
//! ZSHRS-WIDE BUG CLUSTER: `print -P` SGR-attribute + color processing
//! is broken. zsh's `print -P '%B'` outputs the SGR byte sequence
//! (with line-editor ignore markers stripped). zshrs outputs differs
//! consistently across %B/%b/%U/%u/%S/%s/%F{...}/%f/%K{...}/%k/%{...%}.
//! 16 of 26 tests fail; all marked #[ignore] with FIXME. Real-world
//! impact: any zsh user using `print -P` for colored shell output or
//! prompt rendering gets wrong output from zshrs.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") { return PathBuf::from(p); }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("debug").join("zshrs")
}
fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() { "/opt/homebrew/bin/zsh" }
    else if Path::new("/usr/local/bin/zsh").exists() { "/usr/local/bin/zsh" }
    else { "/bin/zsh" }
}
fn zsh_available() -> bool {
    Command::new(zsh_path()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}
struct R { stdout: String, exit: i32 }
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path()).args(["-fc", s]).output().expect("zsh");
    R { stdout: String::from_utf8_lossy(&o.stdout).into_owned(), exit: o.status.code().unwrap_or(-1) }
}
fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin()).args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE").output().expect("zshrs");
    R { stdout: String::from_utf8_lossy(&o.stdout).into_owned(), exit: o.status.code().unwrap_or(-1) }
}
fn assert_parity(s: &str) {
    if !zsh_available() { return; }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}", z.stdout, r.stdout);
    assert_eq!(z.exit, r.exit);
}

mod literals {
    use super::*;

    #[test]
    fn plain_text_unchanged() {
        assert_parity(r#"print -P 'hello world'"#);
    }

    #[test]
    fn double_percent_yields_single() {
        assert_parity(r#"print -P '%%'"#);
    }

    #[test]
    fn empty_string() {
        assert_parity(r#"print -P ''"#);
    }

    #[test]
    fn percent_in_middle() {
        assert_parity(r#"print -P 'a%%b'"#);
    }
}

mod ansi_attrs {
    use super::*;

    #[test]
    fn B_bold_emits_sgr() {
        assert_parity(r#"print -P '%B' | cat -v"#);
    }

    #[test]
    fn b_reset_emits_sgr() {
        assert_parity(r#"print -P '%b' | cat -v"#);
    }

    #[test]
    fn U_underline_emits_sgr() {
        assert_parity(r#"print -P '%U' | cat -v"#);
    }

    #[test]
    fn u_underline_off_emits_sgr() {
        assert_parity(r#"print -P '%u' | cat -v"#);
    }

    #[test]
    fn S_standout_emits_sgr() {
        assert_parity(r#"print -P '%S' | cat -v"#);
    }

    #[test]
    fn s_standout_off_emits_sgr() {
        assert_parity(r#"print -P '%s' | cat -v"#);
    }
}

mod colors {
    use super::*;

    #[test]
    fn F_red_emits_sgr_31() {
        assert_parity(r#"print -P '%F{red}' | cat -v"#);
    }

    #[test]
    fn F_blue_emits_sgr_34() {
        assert_parity(r#"print -P '%F{blue}' | cat -v"#);
    }

    #[test]
    fn f_reset_fg() {
        assert_parity(r#"print -P '%f' | cat -v"#);
    }

    #[test]
    fn K_red_bg_emits_sgr_41() {
        assert_parity(r#"print -P '%K{red}' | cat -v"#);
    }

    #[test]
    fn k_reset_bg() {
        assert_parity(r#"print -P '%k' | cat -v"#);
    }

    #[test]
    fn F_numeric_index_1_is_red() {
        assert_parity(r#"print -P '%F{1}' | cat -v"#);
    }
}

mod literal_opaque {
    use super::*;

    #[test]
    fn literal_braces_content_only() {
        assert_parity(r#"print -P '%{ABCD%}' | cat"#);
    }

    #[test]
    fn literal_braces_with_text_around() {
        assert_parity(r#"print -P 'before%{ABC%}after' | cat"#);
    }
}

mod conditional {
    use super::*;

    /// `%(?.true_branch.false_branch)` — conditional on last exit code.
    #[test]
    fn conditional_dollar_q_true_branch() {
        assert_parity(r#"true; print -P '%(?.OK.FAIL)'"#);
    }

    #[test]
    fn conditional_dollar_q_false_branch() {
        assert_parity(r#"false; print -P '%(?.OK.FAIL)'"#);
    }
}

mod hash_marker {
    use super::*;

    #[test]
    fn hash_marker_non_root_is_percent() {
        assert_parity(r#"print -P '%#'"#);
    }
}

mod text_around_escape {
    use super::*;

    #[test]
    fn text_before_after_bold() {
        assert_parity(r#"print -P 'pre%Bmid%bafter' | cat -v"#);
    }

    #[test]
    fn nested_color_then_attribute() {
        assert_parity(r#"print -P '%F{red}%BHIGHLIGHT%b%f' | cat -v"#);
    }
}

mod unknown_escape {
    use super::*;

    /// Unknown escape doesn't crash either shell.
    #[test]
    fn unknown_letter_handled_safely() {
        // Pin exit code (= 0); output may vary on how unknown is handled.
        assert_parity(r#"print -P '%Q' >/dev/null; echo $?"#);
    }
}

mod no_dash_P_no_processing {
    use super::*;

    /// Without `-P`, `%` chars are literal.
    #[test]
    fn no_P_percent_is_literal() {
        assert_parity(r#"print '%B'"#);
    }

    #[test]
    fn no_P_percent_braces_literal() {
        assert_parity(r#"print '%{abc%}'"#);
    }
}
