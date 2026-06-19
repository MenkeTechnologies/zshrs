//! Function definition + invocation parity tests.

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

mod def_forms {
    use super::*;

    /// `name() { body; }` POSIX form.
    #[test]
    fn paren_form_defines_and_invokes() {
        assert_parity("greet() { echo hello; }; greet");
    }

    /// `function name { body; }` zsh form.
    #[test]
    fn function_keyword_form_defines_and_invokes() {
        assert_parity("function greet { echo hello; }; greet");
    }

    /// `function name() { body; }` mixed form (legal in zsh).
    #[test]
    fn function_keyword_with_parens_works() {
        assert_parity("function greet() { echo hello; }; greet");
    }

    /// Empty body — function exists and does nothing.
    #[test]
    fn function_with_empty_body_returns_zero() {
        assert_parity("f() { :; }; f; echo $?");
    }

    /// Function on one line.
    #[test]
    fn function_inline_one_line() {
        assert_parity("f() { echo one; echo two; }; f");
    }
}

mod arguments {
    use super::*;

    #[test]
    fn dollar_one_dollar_two() {
        assert_parity("f() { echo $1 $2; }; f alpha beta");
    }

    #[test]
    fn dollar_hash_counts_args() {
        assert_parity("f() { echo $#; }; f a b c d");
    }

    #[test]
    fn dollar_at_splats_all_args() {
        assert_parity(r#"f() { for x in "$@"; do echo $x; done; }; f a b c"#);
    }

    #[test]
    fn dollar_star_splats_joined() {
        assert_parity(r#"f() { echo "$*"; }; f a b c"#);
    }

    #[test]
    fn dollar_zero_inside_fn_is_caller_name() {
        // $0 inside a fn defaults to the function name in zsh
        // (unlike bash where it stays the script name).
        assert_parity("greet() { echo $0; }; greet");
    }

    #[test]
    fn many_args() {
        assert_parity("f() { echo $1 $5 $10; }; f a b c d e f g h i j");
    }

    #[test]
    fn shift_drops_first_arg() {
        assert_parity("f() { shift; echo $1; }; f a b c");
    }

    #[test]
    fn shift_two_drops_two() {
        assert_parity("f() { shift 2; echo $1; }; f a b c d");
    }
}

mod return_codes {
    use super::*;

    #[test]
    fn return_zero_default() {
        assert_parity("f() { :; }; f; echo $?");
    }

    #[test]
    fn return_explicit_42() {
        assert_parity("f() { return 42; }; f; echo $?");
    }

    #[test]
    fn return_propagates_last_command() {
        assert_parity("f() { false; }; f; echo $?");
    }

    #[test]
    fn return_in_middle_short_circuits() {
        assert_parity("f() { echo before; return 7; echo after; }; f; echo $?");
    }

    #[test]
    fn return_with_negative_wraps() {
        // C-faithful: return -1 wraps to 255
        assert_parity("f() { return 255; }; f; echo $?");
    }

    /// An EMPTY function body / list resets $? to 0 (c:Src/exec.c:1439-1442
    /// execlist "Empty list; this returns status zero"), not the prior
    /// command's status.
    #[test]
    fn empty_body_resets_status() {
        assert_parity("f() { }; false; f; echo $?");
    }

    #[test]
    fn empty_brace_block_resets_status() {
        assert_parity("false; { }; echo $?");
    }

    #[test]
    fn empty_subshell_resets_status() {
        assert_parity("false; ( ); echo $?");
    }

    #[test]
    fn empty_anon_function_resets_status() {
        assert_parity("false; () { }; echo $?");
    }

    /// Non-empty body still propagates its own status.
    #[test]
    fn nonempty_body_propagates() {
        assert_parity("f() { return 3; }; f; echo $?");
    }
}

mod local_vars {
    use super::*;

    #[test]
    fn local_var_doesnt_leak() {
        assert_parity("f() { local X=inside; echo $X; }; X=outside; f; echo $X");
    }

    #[test]
    fn local_var_shadows_global() {
        assert_parity("X=outer; f() { local X=inner; echo $X; }; f; echo $X");
    }

    #[test]
    fn unset_local_doesnt_unset_global() {
        assert_parity("X=outer; f() { local X; echo before:[$X]; }; f; echo after:[$X]");
    }
}

mod recursion {
    use super::*;

    /// Plain recursion — countdown.
    #[test]
    fn recursive_countdown() {
        assert_parity(
            r#"
countdown() {
  local n=$1
  if (( n <= 0 )); then
    echo done
    return
  fi
  echo $n
  countdown $(( n - 1 ))
}
countdown 3
"#,
        );
    }

    /// Factorial via recursion.
    #[test]
    fn recursive_factorial() {
        assert_parity(
            r#"
fact() {
  if (( $1 <= 1 )); then
    echo 1
  else
    local prev=$(fact $(( $1 - 1 )))
    echo $(( $1 * prev ))
  fi
}
fact 5
"#,
        );
    }
}

mod calling_other_fns {
    use super::*;

    #[test]
    fn a_calls_b() {
        assert_parity("b() { echo b; }; a() { echo a; b; }; a");
    }

    #[test]
    fn fn_calls_fn_with_args() {
        assert_parity(
            r#"
greet() { echo "hi $1"; }
both() { greet $1; greet $2; }
both world friend
"#,
        );
    }
}

mod redefinition {
    use super::*;

    /// Redefining replaces the function body.
    #[test]
    fn redefining_function_replaces_body() {
        assert_parity("f() { echo first; }; f() { echo second; }; f");
    }

    /// `unset -f name` removes function.
    #[test]
    fn unset_f_removes_function() {
        assert_parity("f() { echo hi; }; unset -f f; f 2>/dev/null; echo done");
    }
}

mod fn_in_pipeline {
    use super::*;

    #[test]
    fn function_output_piped_to_grep() {
        assert_parity(r#"f() { echo one; echo two; echo three; }; f | grep t"#);
    }

    #[test]
    fn function_as_data_source_for_while_read() {
        assert_parity(
            r#"
src() { echo a; echo b; echo c; }
src | while read line; do echo got $line; done
"#,
        );
    }
}

mod nested_def {
    use super::*;

    /// Nested function definitions — outer + inner.
    #[test]
    fn nested_function_def_inside_outer() {
        assert_parity(
            r#"
outer() {
  inner() { echo from-inner; }
  inner
}
outer
"#,
        );
    }
}

mod exit_status_propagation {
    use super::*;

    /// Function exit status = last command's exit status.
    #[test]
    fn last_command_status_propagates() {
        assert_parity("f() { true; false; }; f; echo $?");
    }

    /// Function called in condition — exit status decides if/while.
    #[test]
    fn function_in_if_condition() {
        assert_parity("f() { return 0; }; if f; then echo yes; else echo no; fi");
    }

    #[test]
    fn function_returning_nonzero_in_if() {
        assert_parity("f() { return 1; }; if f; then echo yes; else echo no; fi");
    }
}

mod round_ao_pins {
    use super::*;

    #[test]
    fn unfunction_smoke() {
        assert_parity(r#"f_ao(){ :; }; unfunction f_ao 2>/dev/null; echo $?"#);
    }

    #[test]
    fn alias_then_unalias() {
        assert_parity(r#"alias zt='echo z'; zt; unalias zt 2>/dev/null; echo $?"#);
    }

    #[test]
    fn plus_functions_table() {
        assert_parity(r#"print -r ${+functions}"#);
    }

    #[test]
    fn whence_w_print() {
        assert_parity("whence -w print");
    }
}

/// Multi-name function definition `f1 f2 f3() { body }` defines EVERY
/// name with the same body under MULTIFUNCDEF (c:Src/parse.c:2055-2068).
/// The parser previously kept only the last name, leaving the rest
/// undefined.
mod multi_name_funcdef {
    use super::*;

    #[test]
    fn three_names_all_defined() {
        assert_parity("setopt multifuncdef; f1 f2 f3() { print body $0 }; f1; f2; f3");
    }

    #[test]
    fn two_names_share_body() {
        assert_parity("setopt multifuncdef; g1 g2() { print hi $0 }; g1; g2");
    }

    /// Single-name definition still works (regression guard).
    #[test]
    fn single_name_unaffected() {
        assert_parity("solo() { echo single }; solo");
    }

    /// Anonymous function with args still works (regression guard).
    #[test]
    fn anonymous_function_unaffected() {
        assert_parity("() { echo anon $1 } arg1");
    }
}

/// `function NAME () <short-body>` (function keyword + parens + braceless
/// body) parses the body as ONE sublist (c:parse.c:1747 par_list1), not a
/// greedy list. The port used par_list which swallowed the trailing `;`
/// command and errored "parse error near `<next>'".
mod function_keyword_short_body {
    use super::*;

    #[test]
    fn function_paren_short_body() {
        assert_parity("function foo () print bar; foo");
    }

    #[test]
    fn function_paren_adjacent_short_body() {
        assert_parity("function foo() print bar; foo");
    }

    #[test]
    fn short_body_then_following_command() {
        assert_parity("function foo () print bar; echo after");
    }

    #[test]
    fn short_body_compound() {
        assert_parity("function f() for x in 1 2; do echo $x; done; f");
    }

    /// Braced + braceless-no-keyword forms still work (regression guards).
    #[test]
    fn braced_body_unaffected() {
        assert_parity("function foo () { print bar }; foo");
    }

    #[test]
    fn no_keyword_short_body_unaffected() {
        assert_parity("foo() print bar; foo");
    }
}
