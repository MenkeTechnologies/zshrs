//! `eval` builtin parity tests.

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
    fn eval_simple_command() {
        assert_parity(r#"eval echo hello"#);
    }

    #[test]
    fn eval_empty_string() {
        assert_parity(r#"eval ""; echo $?"#);
    }

    /// eval of no args.
    #[test]
    fn eval_no_args() {
        assert_parity(r#"eval; echo $?"#);
    }

    /// eval exit code = exit of last command run.
    #[test]
    fn eval_exit_propagates() {
        assert_parity(r#"eval "false"; echo $?"#);
    }
}

mod multiple_args {
    use super::*;

    /// Args joined by space.
    #[test]
    fn eval_joins_args_by_space() {
        assert_parity(r#"eval echo a b c"#);
    }

    /// Args with quoted spaces.
    #[test]
    fn eval_args_with_quotes() {
        assert_parity(r#"eval 'echo' '"hello world"'"#);
    }

    /// Multiple commands separated by ;.
    #[test]
    fn eval_with_semicolons() {
        assert_parity(r#"eval "echo a; echo b; echo c""#);
    }
}

mod dynamic_var {
    use super::*;

    /// Build var name dynamically.
    #[test]
    fn eval_dynamic_var_assignment() {
        assert_parity(r#"NAME=greet; eval "$NAME=hello"; echo "$greet""#);
    }

    /// Build and call function dynamically.
    #[test]
    fn eval_dynamic_function_call() {
        assert_parity(
            r#"
greet() { echo "hi"; }
FN=greet
eval "$FN"
"#,
        );
    }

    /// Build complex command from variable.
    #[test]
    fn eval_assemble_command() {
        assert_parity(r#"CMD=echo; ARG=hi; eval "$CMD $ARG""#);
    }
}

mod expansion_levels {
    use super::*;

    /// `eval echo \$X` adds an expansion level.
    #[test]
    fn eval_double_expand_with_escaped_dollar() {
        assert_parity(r#"X=value; eval echo \$X"#);
    }

    /// Without escaping, `$X` expands once (in eval's arg list).
    #[test]
    fn eval_single_expand_unescaped() {
        assert_parity(r#"X=value; eval echo $X"#);
    }

    /// Double-nested eval.
    #[test]
    fn eval_nested() {
        assert_parity(r#"X=value; eval eval echo \\\$X"#);
    }
}

mod with_subshell {
    use super::*;

    /// eval in subshell — var assignments don't leak.
    #[test]
    fn eval_in_subshell_isolated() {
        assert_parity(r#"(eval "Y=set"); echo "[$Y]""#);
    }

    /// eval that exits — only affects current shell.
    #[test]
    fn eval_exit_in_subshell() {
        assert_parity(r#"(eval "exit 7"); echo $?"#);
    }
}

mod syntax_errors {
    use super::*;

    /// eval of syntax-error → nonzero exit.
    #[test]
    fn eval_syntax_error_nonzero_exit() {
        assert_parity(r#"eval "if then fi" 2>/dev/null; echo $?"#);
    }

    /// eval of unclosed quote → error.
    #[test]
    fn eval_unclosed_quote_errors() {
        assert_parity(r#"eval 'echo "unterminated' 2>/dev/null; echo $?"#);
    }
}

mod local_scope {
    use super::*;

    /// Vars set inside eval persist in current scope.
    #[test]
    fn eval_assignment_persists() {
        assert_parity(r#"eval "Z=local"; echo $Z"#);
    }

    /// eval inside function honors local scope.
    #[test]
    fn eval_in_function_uses_local_scope() {
        assert_parity(
            r#"
f() {
  local INNER=inner
  eval "echo $INNER"
}
f
"#,
        );
    }
}

mod special_chars_in_arg {
    use super::*;

    /// eval of command with semicolons and quotes.
    #[test]
    fn eval_with_complex_quoting() {
        assert_parity(r#"eval 'X="a b c"; echo "[$X]"'"#);
    }

    /// Embedded newlines.
    #[test]
    fn eval_with_newlines() {
        assert_parity(r#"eval $'echo a\necho b\necho c'"#);
    }

    /// eval of arithmetic expansion.
    #[test]
    fn eval_arith_expansion() {
        assert_parity(r#"eval "echo \$((3+4))""#);
    }
}

mod side_effects {
    use super::*;

    /// eval to set positional params.
    #[test]
    fn eval_set_positional() {
        assert_parity(r#"eval "set -- a b c"; echo "$1/$2/$3""#);
    }

    /// eval to define alias.
    #[test]
    fn eval_define_alias() {
        assert_parity(
            r#"
eval "alias myhi='echo hi'"
alias myhi
"#,
        );
    }

    /// eval to define function.
    #[test]
    fn eval_define_function() {
        assert_parity(
            r#"
eval "myfn() { echo from-fn; }"
myfn
"#,
        );
    }
}

mod return_in_eval {
    use super::*;

    /// `return` inside eval inside function exits function.
    #[test]
    fn return_in_eval_exits_function() {
        assert_parity(
            r#"
f() {
  eval "return 42"
  echo "should not print"
}
f
echo "exit=$?"
"#,
        );
    }
}

mod nested_dollar {
    use super::*;

    /// eval with command substitution.
    #[test]
    fn eval_with_cmd_subst() {
        assert_parity(r#"eval "echo $(echo nested)""#);
    }

    /// eval that produces command substitution.
    #[test]
    fn eval_emits_cmd_subst() {
        assert_parity(r#"eval 'echo $(echo nested)'"#);
    }
}
