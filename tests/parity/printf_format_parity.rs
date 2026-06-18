//! printf format-spec parity tests.

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

mod string_format {
    use super::*;

    #[test]
    fn percent_s_plain() {
        assert_parity(r#"printf "%s" hello"#);
    }

    #[test]
    fn percent_s_with_newline() {
        assert_parity(r#"printf "%s\n" hello"#);
    }

    #[test]
    fn percent_s_width_left_pad() {
        assert_parity(r#"printf "[%10s]\n" hi"#);
    }

    #[test]
    fn percent_s_width_right_pad_via_minus() {
        assert_parity(r#"printf "[%-10s]\n" hi"#);
    }

    #[test]
    fn percent_s_precision_truncates() {
        assert_parity(r#"printf "[%.3s]\n" hello"#);
    }

    #[test]
    fn percent_s_empty_arg() {
        assert_parity(r#"printf "[%s]\n" """#);
    }
}

mod integer_format {
    use super::*;

    #[test]
    fn percent_d_basic() {
        assert_parity(r#"printf "%d\n" 42"#);
    }

    #[test]
    fn percent_d_negative() {
        assert_parity(r#"printf "%d\n" -7"#);
    }

    #[test]
    fn percent_d_zero() {
        assert_parity(r#"printf "%d\n" 0"#);
    }

    #[test]
    fn percent_d_width_pad_with_spaces() {
        assert_parity(r#"printf "[%5d]\n" 42"#);
    }

    #[test]
    fn percent_d_zero_padded() {
        assert_parity(r#"printf "[%05d]\n" 42"#);
    }

    #[test]
    fn percent_d_left_aligned() {
        assert_parity(r#"printf "[%-5d]\n" 42"#);
    }

    #[test]
    fn percent_d_explicit_plus_sign() {
        assert_parity(r#"printf "%+d\n" 42"#);
    }

    #[test]
    fn percent_d_explicit_plus_sign_negative() {
        assert_parity(r#"printf "%+d\n" -42"#);
    }

    #[test]
    fn percent_i_same_as_d() {
        assert_parity(r#"printf "%i\n" 42"#);
    }
}

mod hex_format {
    use super::*;

    #[test]
    fn percent_x_lowercase() {
        assert_parity(r#"printf "%x\n" 255"#);
    }

    #[test]
    fn percent_X_uppercase() {
        assert_parity(r#"printf "%X\n" 255"#);
    }

    #[test]
    fn percent_hash_x_prefix() {
        assert_parity(r#"printf "%#x\n" 255"#);
    }

    #[test]
    fn percent_x_zero() {
        assert_parity(r#"printf "%x\n" 0"#);
    }

    #[test]
    fn percent_x_width_zero_pad() {
        assert_parity(r#"printf "%04x\n" 15"#);
    }
}

mod octal_format {
    use super::*;

    #[test]
    fn percent_o_basic() {
        assert_parity(r#"printf "%o\n" 8"#);
    }

    #[test]
    fn percent_o_zero() {
        assert_parity(r#"printf "%o\n" 0"#);
    }

    #[test]
    fn percent_o_with_hash_prefix() {
        assert_parity(r#"printf "%#o\n" 8"#);
    }
}

mod float_format {
    use super::*;

    #[test]
    fn percent_f_default_precision() {
        assert_parity(r#"printf "%f\n" 3.14"#);
    }

    #[test]
    fn percent_f_precision_two() {
        assert_parity(r#"printf "%.2f\n" 3.14159"#);
    }

    #[test]
    fn percent_f_precision_zero() {
        assert_parity(r#"printf "%.0f\n" 3.7"#);
    }

    #[test]
    fn percent_f_negative() {
        assert_parity(r#"printf "%.1f\n" -2.5"#);
    }

    #[test]
    fn percent_e_scientific() {
        assert_parity(r#"printf "%e\n" 1234.5"#);
    }

    #[test]
    fn percent_g_general() {
        assert_parity(r#"printf "%g\n" 1234.5"#);
    }

    /// NaN/Inf force bare lowercase "nan"/"inf"/"-inf" with no
    /// width/precision/padding (c:builtin.c:5495-5499).
    #[test]
    fn nan_inf_forced_lowercase_f() {
        assert_parity(r#"printf "%.1f|" NaN nan inf Inf -Inf"#);
    }

    #[test]
    fn nan_inf_scientific() {
        assert_parity(r#"printf "%e|%g|" nan inf"#);
    }

    #[test]
    fn inf_ignores_width_padding() {
        assert_parity(r#"printf "[%08.1f][%-8.1f]" inf nan"#);
    }
}

mod escape_sequences {
    use super::*;

    #[test]
    fn escape_n_newline() {
        assert_parity(r#"printf "a\nb\n""#);
    }

    #[test]
    fn escape_t_tab() {
        assert_parity(r#"printf "a\tb\n""#);
    }

    #[test]
    fn escape_r_carriage_return_byte_count() {
        assert_parity(r#"printf "\r" | wc -c"#);
    }

    #[test]
    fn escape_backslash() {
        assert_parity(r#"printf "a\\b\n""#);
    }

    #[test]
    fn escape_double_quote_literal() {
        assert_parity(r#"printf "a\"b\n""#);
    }
}

mod literal_percent {
    use super::*;

    #[test]
    fn double_percent_yields_single() {
        assert_parity(r#"printf "100%%\n""#);
    }

    /// A bare `%` at end of format is an invalid directive (rc 1), not a
    /// literal `%` (c:builtin.c:5430-5436). Preceding output is emitted.
    #[test]
    fn bare_trailing_percent_errors() {
        assert_parity(r#"printf '%'; echo " rc=$?""#);
    }

    #[test]
    fn bare_percent_after_text_errors() {
        assert_parity(r#"printf 'a%' 2>/dev/null; echo " rc=$?""#);
    }
}

mod format_reuse {
    use super::*;

    /// printf re-uses format string for excess args.
    #[test]
    fn format_reuses_for_extra_args() {
        assert_parity(r#"printf "%s\n" a b c"#);
    }

    #[test]
    fn format_with_two_specs_pairs_two_args() {
        assert_parity(r#"printf "%s=%d\n" name 42 age 30"#);
    }

    /// Fewer args than specs — missing → empty/0.
    #[test]
    fn fewer_args_than_specs_fills_default() {
        assert_parity(r#"printf "%s|%s\n" only"#);
    }
}

mod combined_specs {
    use super::*;

    #[test]
    fn percent_d_and_s_mixed() {
        assert_parity(r#"printf "%s has %d items\n" cart 3"#);
    }

    #[test]
    fn padded_pair() {
        assert_parity(r#"printf "[%10s][%-10s]\n" right left"#);
    }
}

mod percent_b {
    use super::*;

    /// `%b` — interpret backslash escapes in ARG (not format).
    #[test]
    fn percent_b_processes_escapes() {
        assert_parity(r#"printf "%b\n" 'a\nb'"#);
    }

    /// `%s` does NOT process escapes.
    #[test]
    fn percent_s_doesnt_process_escapes() {
        assert_parity(r#"printf "%s\n" 'a\nb'"#);
    }

    /// `%b` shares `%s` width+precision, applied to the expansion (c:5307-5360).
    #[test]
    fn percent_b_width() {
        assert_parity(r#"printf "[%5b]\n" ab"#);
    }

    #[test]
    fn percent_b_width_left() {
        assert_parity(r#"printf "[%-5b]\n" ab"#);
    }

    #[test]
    fn percent_b_width_precision() {
        assert_parity(r#"printf "[%3.1b]\n" abcdef"#);
    }
}

mod char_format {
    use super::*;

    /// `%c` — first char of arg.
    #[test]
    fn percent_c_first_char() {
        assert_parity(r#"printf "%c\n" hello"#);
    }

    /// `%c` applies WIDTH/justify but NOT precision (c:5300-5306).
    #[test]
    fn percent_c_width_right() {
        assert_parity(r#"printf "[%3c]\n" x"#);
    }

    #[test]
    fn percent_c_width_left() {
        assert_parity(r#"printf "[%-3c]\n" x"#);
    }

    #[test]
    fn percent_c_precision_ignored() {
        assert_parity(r#"printf "[%.0c]\n" x"#);
    }
}

/// `\c` in the printf format (or in a `%b` arg) truncates ALL output and
/// stops the format-reuse loop (c:builtin.c:5382/5527 fmttrunc).
mod backslash_c_truncation {
    use super::*;

    #[test]
    fn c_in_format_stops_reuse() {
        assert_parity(r#"printf '%s %b!\c%s %s' 1 2 3 4 5 6 7 8 9; echo .EoL."#);
    }

    #[test]
    fn c_in_b_arg_truncates() {
        assert_parity(r#"printf '%s %b!\c%s %s' '1\c' '2\n\c' 3 4 5 6 7 8 9"#);
    }

    #[test]
    fn c_in_late_b_arg_truncates() {
        assert_parity(r#"printf '%b %b\n' 1 2 3 4 '5\c' 6 7 8 9; echo .EoL."#);
    }

    /// No `\c` → format reuse still works.
    #[test]
    fn no_c_reuses_format() {
        assert_parity(r#"printf '%s\n' a b c"#);
    }
}
