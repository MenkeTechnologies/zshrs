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

mod precedence {
    use super::*;

    /// `(( ))` must use zsh's math.c operator precedence, not C precedence.
    /// This mixed bitwise/shift/power expression is 1591 in zsh; the fusevm
    /// ArithCompiler fast path produced 259 (C precedence). Regression guard
    /// for routing precedence-sensitive `(( ))` through MathEval.
    #[test]
    fn mixed_bitwise_shift_power() {
        assert_parity(r#"(( X = 4 - - 3 * 7 << 1 & 7 ^ 1 | 16 ** 2 )); echo $X"#);
    }

    #[test]
    fn or_below_and() {
        assert_parity(r#"(( X = 1 | 2 & 3 )); echo $X"#);
    }

    #[test]
    fn shift_with_bitand() {
        assert_parity(r#"(( X = 4 << 1 & 7 )); echo $X"#);
    }

    #[test]
    fn xor_with_and() {
        assert_parity(r#"(( X = 1 ^ 2 & 3 )); echo $X"#);
    }

    /// The `+ - *` fast path (shared C/zsh precedence) must stay correct.
    #[test]
    fn plus_minus_times_fast_path() {
        assert_parity(r#"(( X = 2 + 3 * 4 - 1 )); echo $X"#);
    }
}

/// `(( $name ))` — the arithmetic COMMAND whose expression text carries a
/// parameter substitution.
///
/// `c:Src/exec.c:5302-5304` is the whole contract:
///
/// ```c
/// e = ecgetstr(state, EC_DUPTOK, &htok);
/// if (htok)
///     singsub(&e);
/// val = matheval(e);
/// ```
///
/// and `c:Src/exec.c:5321` turns the value into the status
/// (`return (val.type == MN_INTEGER) ? val.u.l == 0 : val.u.d == 0.0;`).
/// A math command ALWAYS publishes a status; there is no path out of
/// `execarith` that leaves `lastval` alone.
///
/// zshrs left `$?` untouched for every one of these, so the shell reported
/// whatever the PREVIOUS command had exited with:
///
/// ```text
/// $ x=2; (exit 7); (( $x == 1 )); echo $?
/// zsh   1
/// zshrs 7
/// ```
///
/// which flipped `&&`, ran `until` zero times and `while` one time too many.
mod dollar_in_math_command {
    use super::*;

    /// The base case: a false comparison must exit 1, not inherit.
    #[test]
    fn dollar_scalar_false_comparison_exits_1() {
        assert_parity(r#"x=2; (( $x == 1 )); echo $?"#);
    }

    /// True comparison, so the status is 0 for the right reason rather
    /// than by inheriting a 0.
    #[test]
    fn dollar_scalar_true_comparison_exits_0() {
        assert_parity(r#"x=2; (( $x == 2 )); echo $?"#);
    }

    /// The status must be DERIVED, not inherited — seed `$?` with a value
    /// that neither answer can be confused with.
    #[test]
    fn dollar_status_is_derived_not_inherited() {
        assert_parity(r#"x=2; (exit 7); (( $x == 1 )); echo $?"#);
        assert_parity(r#"x=2; (exit 7); (( $x == 2 )); echo $?"#);
    }

    /// A bare `(( $x ))` on a zero value is false.
    #[test]
    fn dollar_bare_zero_is_false() {
        assert_parity(r#"x=0; (exit 7); (( $x )); echo $?"#);
    }

    /// `${name}` is the same substitution written the other way.
    #[test]
    fn braced_scalar_false_comparison_exits_1() {
        assert_parity(r#"x=2; (( ${x} == 1 )); echo $?"#);
    }

    /// `singsub` expands the TEXT, so a parameter holding an expression
    /// evaluates as that expression.
    #[test]
    fn dollar_expands_text_before_math() {
        assert_parity(r#"x='1+1'; (( $x == 2 )); echo $?"#);
    }

    /// The status feeds `&&` / `||`.
    #[test]
    fn dollar_drives_and_or() {
        assert_parity(r#"x=2; (( $x == 1 )) && echo AND || echo OR"#);
    }

    /// ... and `while`, which ran one iteration too many.
    #[test]
    fn dollar_drives_while() {
        assert_parity(r#"x=2; while (( $x > 0 )); do print -n $x; (( x-- )); done; print"#);
    }

    /// ... and `until`, which ran zero iterations.
    #[test]
    fn dollar_drives_until() {
        assert_parity(r#"x=2; until (( $x == 0 )); do print -n $x; (( x-- )); done; print"#);
    }

    /// A malformed expression still reports the math error and status 2,
    /// which is the branch this status derivation shares.
    #[test]
    fn dollar_math_error_still_exits_2() {
        assert_parity(r#"x=2; (( $x + )) 2>/dev/null; echo $?"#);
    }
}

/// A math command whose value is a FLOAT zero.
///
/// `c:Src/exec.c:5321` distinguishes the two number types:
///
/// ```c
/// return (val.type == MN_INTEGER) ? val.u.l == 0 : val.u.d == 0.0;
/// ```
///
/// The arms of `compile_arith` that hand the expression to the runtime
/// evaluator each derived that status by comparing the RESULT STRING to
/// `"0"`, so a float zero — whose string form is `0.0` — compared unequal
/// and reported true. `(( 0.0 ))` and `(( x ))` were right only because
/// they reach the compiled path, which already calls the primitive that
/// does C's comparison.
mod float_zero_status {
    use super::*;

    /// The `$name` arm.
    #[test]
    fn dollar_float_zero_is_false() {
        assert_parity(r#"x=0.0; (( $x )); echo $?"#);
    }

    /// The `${name}` spelling of the same arm.
    #[test]
    fn braced_float_zero_is_false() {
        assert_parity(r#"x=0.0; (( ${x} )); echo $?"#);
    }

    /// The positional arm, which `arith_uncompilable_reason` sends to the
    /// runtime evaluator for a different reason and which shared the flaw.
    #[test]
    fn positional_float_zero_is_false() {
        assert_parity(r#"set -- 0.0; (( $1 )); echo $?"#);
    }

    /// The subscript arm, likewise.
    #[test]
    fn subscript_float_zero_is_false() {
        assert_parity(r#"a=(0.0); (( ${a[1]} )); echo $?"#);
    }

    /// A float zero produced by the arithmetic rather than read as text.
    #[test]
    fn computed_float_zero_is_false() {
        assert_parity(r#"x=0.0; (( $x + 0.0 )); echo $?"#);
    }

    /// A non-zero float still reads as true.
    #[test]
    fn dollar_float_nonzero_is_true() {
        assert_parity(r#"x=0.5; (( $x )); echo $?"#);
    }

    /// The forms that were already correct must stay correct.
    #[test]
    fn literal_and_bare_float_zero_unchanged() {
        assert_parity(r#"(( 0.0 )); echo $?"#);
        assert_parity(r#"x=0.0; (( x )); echo $?"#);
    }

    /// The read-then-modify arm derives its status from the OLD value, so
    /// a zero slot must stay false — this is what `zinit`'s
    /// `(( ZINIT[SOURCED]++ )) && return` depends on.
    #[test]
    fn subscript_post_increment_from_zero_is_false() {
        assert_parity(r#"typeset -A h; (( h[k]++ )); echo "$? ${h[k]}""#);
        assert_parity(r#"typeset -A h; h[k]=1; (( h[k]++ )); echo "$? ${h[k]}""#);
    }
}
