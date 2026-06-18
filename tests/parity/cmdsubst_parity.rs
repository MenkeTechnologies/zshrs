//! Command substitution `$(cmd)` / backticks + process substitution
//! `<(cmd)` / `>(cmd)` parity tests.

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

mod dollar_paren_subst {
    use super::*;

    #[test]
    fn simple_cmdsubst_in_assignment() {
        assert_parity(r#"X=$(echo hello); echo $X"#);
    }

    #[test]
    fn cmdsubst_inside_echo() {
        assert_parity(r#"echo "result: $(echo embedded)""#);
    }

    #[test]
    fn cmdsubst_with_pipeline_inside() {
        assert_parity(r#"echo $(echo 'a b c' | tr ' ' '\n' | sort)"#);
    }

    /// Trailing newlines are stripped from the result.
    #[test]
    fn cmdsubst_strips_trailing_newlines() {
        assert_parity(r#"X=$(printf 'hi\n\n\n'); echo "[$X]""#);
    }

    /// Internal newlines stay (no leading strip).
    #[test]
    fn cmdsubst_preserves_internal_newlines() {
        assert_parity(r#"echo "[$(printf 'a\nb\nc')]""#);
    }

    /// `$()` of empty command produces empty string.
    #[test]
    fn cmdsubst_of_empty_command_empty_string() {
        assert_parity(r#"X=$(:); echo "[$X]""#);
    }

    /// Multi-line `$()` body.
    #[test]
    fn cmdsubst_multiline_body() {
        assert_parity(
            r#"
X=$(
  echo one
  echo two
)
echo "$X"
"#,
        );
    }
}

mod backticks {
    use super::*;

    #[test]
    fn backtick_cmdsubst_in_assignment() {
        assert_parity(r#"X=`echo hello`; echo $X"#);
    }

    #[test]
    fn backtick_in_echo() {
        assert_parity(r#"echo "got: `echo hi`""#);
    }

    /// Backtick form predates $() — both should behave the same.
    #[test]
    fn backtick_equivalence_to_dollar_paren() {
        assert_parity(r#"echo "[`echo hi`]"; echo "[$(echo hi)]""#);
    }

    /// An UNQUOTED whole-word backtick in argument position IFS
    /// word-splits its output, exactly like `$(...)` (the prior port
    /// only split `$(...)`, leaving `set -- \`echo x y z\`` with $#==1).
    #[test]
    fn backtick_arg_word_splits() {
        assert_parity(r#"print -l `echo a b`"#);
    }

    #[test]
    fn backtick_set_positional_splits() {
        assert_parity(r#"set -- `echo x y z`; echo $#"#);
    }

    #[test]
    fn backtick_split_with_var() {
        assert_parity(r#"foo="two words"; print -l `echo $foo bar`"#);
    }

    /// Quoted backtick does NOT split.
    #[test]
    fn quoted_backtick_no_split() {
        assert_parity(r#"print -l "`echo a b`""#);
    }

    /// Assignment RHS backtick does NOT split.
    #[test]
    fn backtick_assignment_no_split() {
        assert_parity(r#"v=`echo a b c`; print -l $v"#);
    }

    /// Mixed word `a\`cmd\`c` concatenates (not a whole-word backtick).
    #[test]
    fn backtick_mixed_word_concatenates() {
        assert_parity(r#"echo a`echo b`c"#);
    }

    /// Backslash de-escaping inside backticks still works through the
    /// split path (`\$x` → `$x` → expanded at backtick-run time).
    #[test]
    fn backtick_backslash_escape() {
        assert_parity(r#"x=foo; echo `echo \$x`"#);
    }
}

mod nested {
    use super::*;

    #[test]
    fn nested_dollar_paren() {
        assert_parity(r#"echo $(echo $(echo deep))"#);
    }

    #[test]
    fn nested_three_levels() {
        assert_parity(r#"echo $(echo $(echo $(echo bottom)))"#);
    }

    #[test]
    fn cmdsubst_with_var_inside() {
        assert_parity(r#"X=outer; echo "$(echo $X is set)""#);
    }

    #[test]
    fn cmdsubst_arithmetic_inside() {
        assert_parity(r#"echo $(echo $((2+3)))"#);
    }
}

mod with_redirects {
    use super::*;

    /// Inner command stderr discarded — only stdout captured.
    #[test]
    fn cmdsubst_only_captures_stdout() {
        assert_parity(r#"X=$(sh -c 'echo OUT; echo ERR >&2' 2>/dev/null); echo "[$X]""#);
    }

    /// stderr can be merged via 2>&1.
    #[test]
    fn cmdsubst_merge_stderr_via_2_to_1() {
        assert_parity(r#"X=$(sh -c 'echo OUT; echo ERR >&2' 2>&1); echo "[$X]""#);
    }
}

mod word_splitting {
    use super::*;

    /// Unquoted `$(...)` undergoes word splitting on $IFS.
    #[test]
    fn unquoted_cmdsubst_word_splits() {
        assert_parity(r#"f() { echo $#; }; f $(echo a b c)"#);
    }

    /// Quoted `"$(...)"` does NOT word-split.
    #[test]
    fn quoted_cmdsubst_no_word_split() {
        assert_parity(r#"f() { echo $#; }; f "$(echo a b c)""#);
    }
}

mod in_arithmetic {
    use super::*;

    #[test]
    fn cmdsubst_in_arith_context() {
        assert_parity(r#"echo $(( $(echo 5) + 3 ))"#);
    }
}

mod process_subst_in {
    use super::*;

    /// `<(cmd)` — pass command output as a readable file path.
    #[test]
    fn process_subst_in_with_cat() {
        assert_parity(r#"cat <(echo from-procsubst)"#);
    }

    #[test]
    fn process_subst_in_with_diff() {
        // `diff <(echo a) <(echo a)` should produce no diff (exit 0).
        assert_parity(r#"diff <(echo a) <(echo a); echo $?"#);
    }

    #[test]
    fn process_subst_in_with_two_different_inputs() {
        // `diff <(echo a) <(echo b)` should report a diff (exit 1).
        assert_parity(r#"diff <(echo a) <(echo b) >/dev/null; echo $?"#);
    }
}

mod process_subst_out {
    use super::*;

    /// `>(cmd)` — pipe to a command as a writable file path.
    #[test]
    fn process_subst_out_with_tee() {
        assert_parity(r#"echo "data" | tee >(cat > /dev/null) >/dev/null; echo done"#);
    }
}

mod chained {
    use super::*;

    /// Multiple subs in a single command.
    #[test]
    fn multiple_substs_in_one_command() {
        assert_parity(r#"echo "$(echo a) $(echo b) $(echo c)""#);
    }

    #[test]
    fn cmdsubst_in_for_loop_words() {
        assert_parity(r#"for x in $(echo a b c); do echo $x; done"#);
    }

    #[test]
    fn cmdsubst_as_array_init() {
        assert_parity(r#"arr=($(echo a b c)); print -l "${arr[@]}""#);
    }
}

/// c:Src/exec.c:5025 getproc (PATH_DEV_FD) — `>(cmd)` is a pipe whose
/// write end the parent exposes as `/dev/fd/N` and closes when the
/// consuming job finishes. The old FIFO port blocked the child in
/// open(2) before running cmd, so `a=$(print -r -- >(true))` never
/// EOF'd the capture pipe (shell hang).
mod process_subst_out_dev_fd {
    use super::*;

    #[test]
    fn procsubst_out_under_cmdsubst_does_not_hang() {
        assert_parity(r#"a=$(print -r -- >(true)); print done"#);
    }

    #[test]
    fn procsubst_out_path_is_dev_fd() {
        assert_parity(r#"[[ $(print -r -- >(true)) == /dev/fd/* ]] && print devfd"#);
    }

    #[test]
    fn procsubst_out_write_end_closes_after_command() {
        // wc's stdin EOFs only when the parent's write end closes
        // after tee finishes (c: addfilelist → deletefilelist).
        assert_parity(
            r#"t=$(mktemp); tee >(wc -c >$t) </dev/null >/dev/null; sleep 0.2; cat $t; command rm -f $t"#,
        );
    }

    #[test]
    fn procsubst_out_receives_piped_data() {
        assert_parity(
            r#"t=$(mktemp); print -n abcde | tee >(wc -c >$t) >/dev/null; sleep 0.2; cat $t; command rm -f $t"#,
        );
    }
}
