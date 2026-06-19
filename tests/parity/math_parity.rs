//! Arithmetic expansion parity tests — pin `$(( expr ))` and `(( expr ))`
//! against real zsh 5.9.
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

#[allow(dead_code)]
struct ShellResult {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit divergence on script:\n{script}");
}

mod literals {
    use super::*;

    #[test]
    fn integer_literal() {
        assert_parity("echo $((42))");
    }

    #[test]
    fn negative_literal() {
        assert_parity("echo $((-7))");
    }

    #[test]
    fn hex_literal_lowercase() {
        assert_parity("echo $((0xff))");
    }

    #[test]
    fn hex_literal_uppercase() {
        assert_parity("echo $((0XDEAD))");
    }

    #[test]
    fn base_hash_hex() {
        assert_parity("echo $((16#FF))");
    }

    #[test]
    fn base_hash_binary() {
        assert_parity("echo $((2#1010))");
    }

    #[test]
    fn base_hash_octal() {
        assert_parity("echo $((8#17))");
    }

    /// `010` in zsh default is DECIMAL 10 (not octal). Pin.
    #[test]
    fn leading_zero_decimal_default() {
        assert_parity("echo $((010))");
    }

    #[test]
    fn float_literal() {
        assert_parity("echo $((3.14))");
    }
}

mod binary_ops {
    use super::*;

    #[test]
    fn add() {
        assert_parity("echo $((1 + 2))");
    }

    #[test]
    fn sub() {
        assert_parity("echo $((10 - 3))");
    }

    #[test]
    fn mul() {
        assert_parity("echo $((6 * 7))");
    }

    #[test]
    fn div_integer() {
        assert_parity("echo $((17 / 5))");
    }

    #[test]
    fn div_integer_below_one() {
        assert_parity("echo $((1 / 4))");
    }

    #[test]
    fn mod_op() {
        assert_parity("echo $((17 % 5))");
    }

    #[test]
    fn power() {
        assert_parity("echo $((2 ** 10))");
    }

    #[test]
    fn power_cubed() {
        assert_parity("echo $((3 ** 3))");
    }

    #[test]
    fn unary_minus_power() {
        assert_parity("echo $((-2 ** 2))");
    }

    #[test]
    fn paren_unary_minus_power() {
        assert_parity("echo $(((-2) ** 2))");
    }
}

mod precedence {
    use super::*;

    #[test]
    fn mul_over_add() {
        assert_parity("echo $((2 + 3 * 4))");
    }

    #[test]
    fn parens_override() {
        assert_parity("echo $(((2 + 3) * 4))");
    }

    #[test]
    fn nested_parens() {
        assert_parity("echo $(((1 + 2) * (3 + 4)))");
    }

    #[test]
    fn chain_add() {
        assert_parity("echo $((1 + 2 + 3 + 4 + 5))");
    }

    #[test]
    fn chain_sub_left_assoc() {
        assert_parity("echo $((100 - 1 - 2 - 3))");
    }
}

mod bitwise {
    use super::*;

    #[test]
    fn bitand() {
        assert_parity("echo $((0xff & 0x0f))");
    }

    #[test]
    fn bitor() {
        assert_parity("echo $((0xff | 0x100))");
    }

    #[test]
    fn bitxor() {
        assert_parity("echo $((0xff ^ 0x0f))");
    }

    #[test]
    fn bitnot_zero() {
        assert_parity("echo $((~0))");
    }

    #[test]
    fn shift_left() {
        assert_parity("echo $((1 << 8))");
    }

    #[test]
    fn shift_right() {
        assert_parity("echo $((256 >> 4))");
    }

    #[test]
    fn arithmetic_right_shift_preserves_sign() {
        assert_parity("echo $((-1 >> 1))");
    }
}

mod comparison_logical {
    use super::*;

    #[test]
    fn eq_true() {
        assert_parity("echo $((5 == 5))");
    }

    #[test]
    fn eq_false() {
        assert_parity("echo $((5 == 6))");
    }

    #[test]
    fn ne_true() {
        assert_parity("echo $((5 != 6))");
    }

    #[test]
    fn lt_true() {
        assert_parity("echo $((3 < 5))");
    }

    #[test]
    fn le_on_equal() {
        assert_parity("echo $((5 <= 5))");
    }

    #[test]
    fn gt_true() {
        assert_parity("echo $((5 > 3))");
    }

    #[test]
    fn ge_on_equal() {
        assert_parity("echo $((5 >= 5))");
    }

    #[test]
    fn logand_both_true() {
        assert_parity("echo $((1 && 1))");
    }

    #[test]
    fn logand_short_circuit_false() {
        assert_parity("echo $((0 && 99))");
    }

    #[test]
    fn logor_one_true() {
        assert_parity("echo $((0 || 1))");
    }

    #[test]
    fn lognot_zero_is_one() {
        assert_parity("echo $((!0))");
    }

    #[test]
    fn lognot_nonzero_is_zero() {
        assert_parity("echo $((!42))");
    }
}

mod ternary {
    use super::*;

    #[test]
    fn ternary_true_branch() {
        assert_parity("echo $((1 ? 10 : 20))");
    }

    #[test]
    fn ternary_false_branch() {
        assert_parity("echo $((0 ? 10 : 20))");
    }

    #[test]
    fn ternary_with_compare_cond() {
        assert_parity("echo $(((3 < 5) ? 100 : 200))");
    }
}

mod comma_and_assignment {
    use super::*;

    #[test]
    fn comma_returns_last() {
        assert_parity("echo $(((1,2,3)))");
    }

    #[test]
    fn assign_and_use() {
        assert_parity("echo $((A=5, A * 2))");
    }

    #[test]
    fn assign_then_read() {
        assert_parity("(( B = 7 + 3 )); echo $B");
    }

    #[test]
    fn add_assign_op() {
        assert_parity("C=10; (( C += 5 )); echo $C");
    }

    #[test]
    fn sub_assign_op() {
        assert_parity("D=10; (( D -= 3 )); echo $D");
    }

    #[test]
    fn mul_assign_op() {
        assert_parity("E=6; (( E *= 7 )); echo $E");
    }
}

mod inc_dec {
    use super::*;

    #[test]
    fn post_increment_returns_old() {
        assert_parity("X=10; echo $((X++)); echo $X");
    }

    #[test]
    fn pre_increment_returns_new() {
        assert_parity("X=10; echo $((++X)); echo $X");
    }

    #[test]
    fn post_decrement_returns_old() {
        assert_parity("X=10; echo $((X--)); echo $X");
    }

    #[test]
    fn pre_decrement_returns_new() {
        assert_parity("X=10; echo $((--X)); echo $X");
    }
}

mod arith_command_exit {
    use super::*;

    /// `(( expr ))` exits 0 if expr is nonzero, 1 if zero.
    #[test]
    fn arith_cmd_nonzero_exits_zero() {
        assert_parity("(( 1 )); echo $?");
    }

    #[test]
    fn arith_cmd_zero_exits_one() {
        assert_parity("(( 0 )); echo $?");
    }

    #[test]
    fn arith_cmd_compare_true() {
        assert_parity("(( 5 > 3 )); echo $?");
    }

    #[test]
    fn arith_cmd_compare_false() {
        assert_parity("(( 5 < 3 )); echo $?");
    }
}

mod literals_and_bases {
    use super::*;

    #[test]
    fn underscore_in_integer_literal() {
        assert_parity("echo $((1_000 + 2_000))");
    }

    #[test]
    fn base_indicator_hash_hash_a() {
        assert_parity("echo $((##a))");
    }

    #[test]
    fn assign_with_hash_base() {
        assert_parity("(( x = 5#101 )); echo $x");
    }

    #[test]
    fn print_base_five_value() {
        assert_parity("echo $((5#101))");
    }

    #[test]
    fn base_twelve_nine_b() {
        assert_parity("echo $((12#9b))");
    }

    #[test]
    fn binary_literal_0b() {
        assert_parity("echo $((0b101010))");
    }
}

mod power_and_bitwise {
    use super::*;

    #[test]
    fn power_right_associative() {
        assert_parity("echo $((2 ** 3 ** 2))");
    }

    #[test]
    fn bitand_then_xor() {
        assert_parity("echo $((9 & 6 ^ 3))");
    }

    #[test]
    fn chained_shift_right() {
        assert_parity("echo $((128 >> 4 >> 1))");
    }

    #[test]
    fn complement_and_mask() {
        assert_parity("echo $((~(255) & 0xff))");
    }

    #[test]
    fn hex_mask() {
        assert_parity("echo $((0xabc & 0xf0))");
    }
}

mod float_in_arith {
    use super::*;

    #[test]
    fn float_multiply() {
        assert_parity("float f1=1.5 f2=2; echo $((f1 * f2))");
    }

    #[test]
    fn typeset_F_precision() {
        assert_parity("typeset -F2 f=3.14159; echo $f");
    }

    #[test]
    fn typeset_F1_compare() {
        assert_parity("typeset -F1 cmp=1.05; echo $((cmp > 1))");
    }
}

mod compound_assign {
    use super::*;

    #[test]
    fn or_assign() {
        assert_parity("integer i=5; (( i |= 3 )); echo $i");
    }

    #[test]
    fn and_assign() {
        assert_parity("integer i=5; (( i &= 3 )); echo $i");
    }

    #[test]
    fn xor_assign() {
        assert_parity("integer i=5; (( i ^= 3 )); echo $i");
    }

    #[test]
    fn shl_assign() {
        assert_parity("integer i=5; (( i <<= 1 )); echo $i");
    }

    #[test]
    fn shr_assign() {
        assert_parity("integer i=5; (( i >>= 1 )); echo $i");
    }

    #[test]
    fn add_assign() {
        assert_parity("integer i=5; (( i += 3 )); echo $i");
    }

    #[test]
    fn sub_assign() {
        assert_parity("integer i=5; (( i -= 2 )); echo $i");
    }

    #[test]
    fn mul_assign() {
        assert_parity("integer i=5; (( i *= 2 )); echo $i");
    }

    #[test]
    fn div_assign() {
        assert_parity("integer i=5; (( i /= 2 )); echo $i");
    }

    #[test]
    fn mod_assign() {
        assert_parity("integer i=5; (( i %= 3 )); echo $i");
    }
}

mod true_false_in_arith {
    use super::*;

    #[test]
    fn true_is_one() {
        assert_parity("echo $((true))");
    }

    #[test]
    fn false_is_zero() {
        assert_parity("echo $((false))");
    }
}

mod base_indicators {
    use super::*;

    #[test]
    fn hash_hash_Z() {
        assert_parity("echo $((##Z))");
    }

    #[test]
    fn hash_hash_b() {
        assert_parity("echo $((##b))");
    }

    #[test]
    fn binary_0b_literal() {
        assert_parity("echo $((0b1111))");
    }

    #[test]
    fn hex_mixed_case() {
        assert_parity("echo $((0xffFF))");
    }
}

mod with_parameters {
    use super::*;

    #[test]
    fn dollar_var_in_arith() {
        assert_parity("X=42; echo $((X))");
    }

    #[test]
    fn dollar_var_arith() {
        assert_parity("X=10; Y=3; echo $((X + Y))");
    }

    #[test]
    fn unset_var_treated_as_zero() {
        assert_parity("echo $((UNDEFINED + 5))");
    }

    #[test]
    fn integer_typeset_arith() {
        assert_parity("typeset -i X=10; X=X+5; echo $X");
    }

    #[test]
    fn nested_arith_substitution() {
        assert_parity("X=5; echo $(($((X)) * 2))");
    }
}

mod round_pins {
    use super::*;

    #[test]
    fn hex_add() {
        assert_parity("print -r $(( 0x10 + 0x20 ))");
    }

    #[test]
    fn binary_add() {
        assert_parity("print -r $(( 2#101 + 1 ))");
    }

    #[test]
    fn compound_shift_left() {
        assert_parity("integer x=3; (( x <<= 1 )); print -r $x");
    }

    #[test]
    fn compound_shift_right() {
        assert_parity("integer x=5; (( x >>= 1 )); print -r $x");
    }

    #[test]
    fn c_style_for_loop() {
        assert_parity("for ((i=1;i<=3;i++)); do print -r $i; done");
    }

    /// `##X` char-code operator: plain char, and the GETKEY_EMACS escapes
    /// `\C-x` (control) / `\M-x` (meta) decoded via getkeystring (math.c:856).
    #[test]
    fn charcode_plain() {
        assert_parity("print $(( ##A ))");
    }

    #[test]
    fn charcode_control_escape() {
        assert_parity(r#"print $(( ##\C-a ))"#);
    }

    #[test]
    fn charcode_meta_escape() {
        assert_parity(r#"print $(( ##\M-a ))"#);
    }

    #[test]
    fn charcode_named_escape() {
        assert_parity(r#"print $(( ##\n ))"#);
    }

    /// `$((##))` with nothing after the marker is a math error (exit nonzero),
    /// not 0 (math.c:852-854).
    #[test]
    fn charcode_missing_char_errors() {
        assert_parity(r#"print $((##))"#);
    }

    /// `forcefloat` promotes integer arithmetic to float (math.c:348/359).
    #[test]
    fn force_float_division() {
        assert_parity("setopt force_float; print $(( 3/4 ))");
    }

    #[test]
    fn force_float_integer_literal() {
        assert_parity("setopt force_float; print $(( 1 ))");
    }

    /// forcefloat coerces INTEGER PARAM reads to float too (c:math.c:359)
    /// — `integer a=3 b=4; $((a/b))` → 0.75, not 0. The param-read path
    /// previously dropped the coercion (literals already had it).
    #[test]
    fn force_float_integer_param_division() {
        assert_parity("integer a=3 b=4; setopt force_float; print $(( a/b ))");
    }

    #[test]
    fn force_float_integer_param_value() {
        assert_parity("integer a=10; setopt force_float; print $(( a ))");
    }

    /// Without force_float, integer params divide as integers.
    #[test]
    fn no_force_float_integer_param_division() {
        assert_parity("integer a=3 b=4; print $(( a/b ))");
    }

    /// `cprecedences` switches `(( ))` to C operator precedence (259 vs 1591).
    #[test]
    fn cprecedences_uses_c_precedence() {
        assert_parity("setopt cprecedences; print $(( 4 - - 3 * 7 << 1 & 7 ^ 1 | 16 ** 2 ))");
    }

    /// `[#base_N]` underscore grouping must group only the digits, leaving the
    /// `0x` / `N#` base prefix untouched (params.c:5654-5657).
    #[test]
    fn underscore_grouping_hex_prefix() {
        assert_parity("setopt cbases; print $(( [#16_] 65536 ))");
    }

    #[test]
    fn underscore_grouping_base_prefix() {
        assert_parity("print $(( [#2_4] 255 ))");
    }

    #[test]
    fn underscore_grouping_decimal() {
        assert_parity("print $(( [#_] 1000000 ))");
    }
}

/// `zsh/mathfunc` `atan` takes 1 OR 2 args: 2-arg form is atan2(y,x)
/// (c:mathfunc.c:225-229, NUMMATHFUNC 1..2). 3+ args error "wrong
/// number of arguments" (c:math.c:1106-1127). The port returned
/// atan(arg1) for 2 args and silently dropped extra args.
mod mathfunc_atan_arity {
    use super::*;

    #[test]
    fn atan_two_args_is_atan2() {
        assert_parity("zmodload zsh/mathfunc; float -F 5 r; (( r = atan(3,2) )); print $r");
    }

    #[test]
    fn atan_one_arg() {
        assert_parity("zmodload zsh/mathfunc; print $(( atan(1) ))");
    }

    #[test]
    fn atan_three_args_errors() {
        assert_parity("zmodload zsh/mathfunc; print $(( atan(1,2,3) )); echo rc=$?");
    }

    /// One-arg funcs still reject a second arg.
    #[test]
    fn sin_two_args_errors() {
        assert_parity("zmodload zsh/mathfunc; print $(( sin(0,1) )) 2>/dev/null; echo rc=$?");
    }

    /// Two-arg func still works (regression guard).
    #[test]
    fn fmod_two_args() {
        assert_parity("zmodload zsh/mathfunc; print $(( fmod(10,3) ))");
    }
}
