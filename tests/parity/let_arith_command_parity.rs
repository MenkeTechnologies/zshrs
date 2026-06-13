//! `let` builtin + `(( ))` command-form arithmetic parity.
//!
//! Key semantic: `let` and `(( ))` exit 0 if last expression is nonzero,
//! exit 1 if zero. This is shell-arithmetic-as-boolean semantics, used
//! for `if (( x > 0 )); then ...; fi`.

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

mod paren_command {
    use super::*;

    /// `(( expr ))` is true (exit 0) when expr is nonzero.
    #[test]
    fn paren_nonzero_exit_0() {
        assert_parity(r#"(( 5 )); echo $?"#);
    }

    /// `(( expr ))` is false (exit 1) when expr is zero.
    #[test]
    fn paren_zero_exit_1() {
        assert_parity(r#"(( 0 )); echo $?"#);
    }

    /// Negative also non-false → exit 0.
    #[test]
    fn paren_negative_exit_0() {
        assert_parity(r#"(( -5 )); echo $?"#);
    }

    /// `if (( expr ))` works as conditional.
    #[test]
    fn if_paren_comparison() {
        assert_parity(r#"X=5; if (( X > 3 )); then echo big; else echo small; fi"#);
    }

    #[test]
    fn if_paren_comparison_false() {
        assert_parity(r#"X=1; if (( X > 3 )); then echo big; else echo small; fi"#);
    }

    /// `while (( i > 0 ))` loop.
    #[test]
    fn while_paren_countdown() {
        assert_parity(r#"i=3; while (( i > 0 )); do echo $i; (( i-- )); done"#);
    }
}

mod let_builtin {
    use super::*;

    /// `let X=5` sets X.
    #[test]
    fn let_assignment_basic() {
        assert_parity(r#"let X=5; echo $X"#);
    }

    /// `let X=5 Y=10` multi-assignment.
    #[test]
    fn let_multi_assignment() {
        assert_parity(r#"let X=5 Y=10; echo "$X/$Y""#);
    }

    /// let exits 0 if last result is nonzero.
    #[test]
    fn let_exit_0_when_last_nonzero() {
        assert_parity(r#"let "X=5"; echo $?"#);
    }

    /// let exits 1 if last is zero.
    #[test]
    fn let_exit_1_when_last_zero() {
        assert_parity(r#"let "X=0"; echo $?"#);
    }

    /// `let X+=3` compound assignment.
    #[test]
    fn let_compound_plus_eq() {
        assert_parity(r#"X=10; let X+=3; echo $X"#);
    }

    #[test]
    fn let_compound_times_eq() {
        // `*=` must be quoted to avoid glob expansion on the asterisk.
        assert_parity(r#"X=4; let "X*=3"; echo $X"#);
    }
}

mod increment_decrement {
    use super::*;

    /// Post-increment X++ returns old value but mutates.
    #[test]
    fn post_increment_value() {
        assert_parity(r#"X=5; (( Y = X++ )); echo "X=$X Y=$Y""#);
    }

    /// Pre-increment ++X returns new value.
    #[test]
    fn pre_increment_value() {
        assert_parity(r#"X=5; (( Y = ++X )); echo "X=$X Y=$Y""#);
    }

    #[test]
    fn post_decrement_value() {
        assert_parity(r#"X=5; (( Y = X-- )); echo "X=$X Y=$Y""#);
    }

    #[test]
    fn pre_decrement_value() {
        assert_parity(r#"X=5; (( Y = --X )); echo "X=$X Y=$Y""#);
    }
}

mod multi_expr {
    use super::*;

    /// Comma operator — result is last.
    #[test]
    fn comma_operator_returns_last() {
        assert_parity(r#"(( X = (1, 2, 3) )); echo $X"#);
    }

    /// Multiple statements via comma in let.
    #[test]
    fn let_with_multiple_expressions() {
        assert_parity(r#"let "X=1, Y=2, Z=X+Y"; echo "$X/$Y/$Z""#);
    }
}

mod ternary {
    use super::*;

    #[test]
    fn ternary_true_branch() {
        assert_parity(r#"X=10; (( Y = X > 5 ? 100 : 200 )); echo $Y"#);
    }

    #[test]
    fn ternary_false_branch() {
        assert_parity(r#"X=3; (( Y = X > 5 ? 100 : 200 )); echo $Y"#);
    }
}

mod logical {
    use super::*;

    /// && short-circuits.
    #[test]
    fn logical_and_both_true() {
        assert_parity(r#"(( 1 && 1 )); echo $?"#);
    }

    #[test]
    fn logical_and_first_false() {
        assert_parity(r#"(( 0 && 1 )); echo $?"#);
    }

    #[test]
    fn logical_or_first_true() {
        assert_parity(r#"(( 1 || 0 )); echo $?"#);
    }

    #[test]
    fn logical_or_both_false() {
        assert_parity(r#"(( 0 || 0 )); echo $?"#);
    }

    /// Logical NOT.
    #[test]
    fn logical_not_zero() {
        assert_parity(r#"(( !0 )); echo $?"#);
    }

    #[test]
    fn logical_not_one() {
        assert_parity(r#"(( !1 )); echo $?"#);
    }
}

mod bitwise {
    use super::*;

    #[test]
    fn bit_and() {
        assert_parity(r#"(( X = 6 & 3 )); echo $X"#);
    }

    #[test]
    fn bit_or() {
        assert_parity(r#"(( X = 6 | 3 )); echo $X"#);
    }

    #[test]
    fn bit_xor() {
        assert_parity(r#"(( X = 6 ^ 3 )); echo $X"#);
    }

    #[test]
    fn left_shift() {
        assert_parity(r#"(( X = 1 << 4 )); echo $X"#);
    }

    #[test]
    fn right_shift() {
        assert_parity(r#"(( X = 32 >> 2 )); echo $X"#);
    }

    #[test]
    fn bit_complement() {
        assert_parity(r#"(( X = ~0 )); echo $X"#);
    }
}

mod power_modulo {
    use super::*;

    /// Power **.
    #[test]
    fn power_operator() {
        assert_parity(r#"(( X = 2 ** 10 )); echo $X"#);
    }

    #[test]
    fn modulo_operator() {
        assert_parity(r#"(( X = 17 % 5 )); echo $X"#);
    }

    /// Modulo negative.
    #[test]
    fn modulo_negative() {
        assert_parity(r#"(( X = -17 % 5 )); echo $X"#);
    }

    /// Integer division.
    #[test]
    fn integer_division() {
        assert_parity(r#"(( X = 17 / 5 )); echo $X"#);
    }
}

mod errors {
    use super::*;

    /// Division by zero → arith error, nonzero exit.
    #[test]
    fn division_by_zero_errors() {
        assert_parity(r#"(( X = 5 / 0 )) 2>/dev/null; echo exit=$?"#);
    }

    /// Modulo by zero → arith error.
    #[test]
    fn modulo_by_zero_errors() {
        assert_parity(r#"(( X = 5 % 0 )) 2>/dev/null; echo exit=$?"#);
    }
}

mod base_literals {
    use super::*;

    /// 0x hex literal.
    #[test]
    fn hex_literal() {
        assert_parity(r#"(( X = 0xff )); echo $X"#);
    }

    /// 0 octal literal.
    #[test]
    fn octal_literal() {
        assert_parity(r#"(( X = 010 )); echo $X"#);
    }

    /// Base#NNN — zsh-specific.
    #[test]
    fn base_2_binary() {
        assert_parity(r#"(( X = 2#1010 )); echo $X"#);
    }

    #[test]
    fn base_16_hex() {
        assert_parity(r#"(( X = 16#ff )); echo $X"#);
    }

    #[test]
    fn base_36_alphanumeric() {
        assert_parity(r#"(( X = 36#z )); echo $X"#);
    }
}
