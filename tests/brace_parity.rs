//! Brace-expansion parity tests — `{a,b,c}`, `{N..M}`, escaped, nested.

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
    assert_eq!(z.exit, r.exit, "exit divergence on:\n{s}");
}

mod comma_list {
    use super::*;

    #[test]
    fn three_alternatives() {
        assert_parity("print -l {a,b,c}");
    }

    #[test]
    fn two_alternatives() {
        assert_parity("print -l {x,y}");
    }

    #[test]
    fn five_alternatives() {
        assert_parity("print -l {a,b,c,d,e}");
    }

    #[test]
    fn with_prefix() {
        assert_parity("print -l pre{a,b,c}");
    }

    #[test]
    fn with_suffix() {
        assert_parity("print -l {a,b,c}post");
    }

    #[test]
    fn with_prefix_and_suffix() {
        assert_parity("print -l pre{a,b,c}post");
    }

    #[test]
    fn empty_alternative() {
        assert_parity("print -l {a,,c}");
    }

    #[test]
    fn single_alternative_no_comma_literal() {
        // `{a}` with no comma — zsh emits literal `{a}`.
        assert_parity("print -l {a}");
    }

    #[test]
    fn double_alternatives_cartesian() {
        assert_parity("print -l {a,b}{c,d}");
    }

    #[test]
    fn triple_alternatives_cartesian() {
        assert_parity("print -l {a,b}{c,d}{e,f}");
    }
}

mod numeric_ranges {
    use super::*;

    #[test]
    fn one_to_five() {
        assert_parity("print -l {1..5}");
    }

    #[test]
    fn five_to_one_reverse() {
        assert_parity("print -l {5..1}");
    }

    #[test]
    fn negative_to_positive() {
        assert_parity("print -l {-3..3}");
    }

    #[test]
    fn step_two() {
        assert_parity("print -l {1..10..2}");
    }

    #[test]
    fn step_three_descending() {
        assert_parity("print -l {10..1..3}");
    }

    #[test]
    fn single_element_range() {
        assert_parity("print -l {3..3}");
    }
}

mod zero_padding {
    use super::*;

    #[test]
    fn two_digit_pad() {
        assert_parity("print -l {01..10}");
    }

    #[test]
    fn three_digit_pad() {
        assert_parity("print -l {001..010}");
    }

    #[test]
    fn pad_with_step() {
        assert_parity("print -l {01..09..2}");
    }
}

mod alpha_ranges {
    use super::*;

    #[test]
    fn lowercase_a_to_e() {
        assert_parity("print -l {a..e}");
    }

    #[test]
    fn lowercase_e_to_a_reverse() {
        assert_parity("print -l {e..a}");
    }

    #[test]
    fn uppercase_A_to_E() {
        assert_parity("print -l {A..E}");
    }

    /// zsh does NOT expand alpha-range with step — emits literal.
    #[test]
    fn alpha_with_step_literal() {
        assert_parity("print -l {a..z..3}");
    }
}

mod escapes_and_quotes {
    use super::*;

    /// Escaped braces — zsh emits literal text.
    #[test]
    fn backslash_braces_literal() {
        assert_parity(r#"print -l \{a,b,c\}"#);
    }

    /// Single-quoted braces — literal.
    #[test]
    fn single_quoted_braces_literal() {
        assert_parity(r#"print -l '{a,b,c}'"#);
    }

    /// Double-quoted braces — literal (braces don't expand in DQ).
    #[test]
    fn double_quoted_braces_literal() {
        assert_parity(r#"print -l "{a,b,c}""#);
    }

    /// Empty braces — literal.
    #[test]
    fn empty_braces_literal() {
        assert_parity("print -l {}");
    }
}

mod nested {
    use super::*;

    #[test]
    fn nested_inside_outer() {
        assert_parity("print -l {a,{b,c}}");
    }

    #[test]
    fn nested_with_range() {
        assert_parity("print -l {a,{1..3}}");
    }

    #[test]
    fn nested_inside_cartesian() {
        assert_parity("print -l {a,b}{x,{y,z}}");
    }
}

mod round_pins {
    use super::*;

    #[test]
    fn numeric_step_range() {
        assert_parity("print -r {1..5..2}");
    }

    #[test]
    fn alpha_range() {
        assert_parity("print -r {a..d}");
    }

    #[test]
    fn zero_padded_range() {
        assert_parity("print -r {01..05}");
    }

    #[test]
    fn leading_comma_empty_first() {
        assert_parity("print -r {,a}");
    }

    #[test]
    fn trailing_comma_suffix() {
        assert_parity("print -r a{,b}");
    }
}

mod combinations {
    use super::*;

    /// Brace expansion happens BEFORE param expansion. Plain literal pre/suffix
    /// with brace list.
    #[test]
    fn brace_first_then_literal_suffix() {
        assert_parity("print -l file{1,2,3}.txt");
    }

    #[test]
    fn brace_after_dollar_var_literal() {
        assert_parity(r#"X=foo; print -l "$X"{a,b,c}"#);
    }

    #[test]
    fn deeply_nested_full_combination() {
        assert_parity("print -l {a,b}/{x,y}/{1,2}");
    }
}
