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

    /// `$((` that is not valid math is re-read as a command substitution
    /// holding a subshell (c:Src/lex.c:520-531): `$(( a[ ))` runs `( a[ )`,
    /// which fails on the bad pattern and substitutes nothing. The math scan
    /// must stop ON the `)` that closes nothing (c:1492-1494, c:1603-1605) and
    /// hand it back to be re-read (c:521). zshrs let that `)` through its loop
    /// head and then dropped the character the scan stopped on, so the
    /// substitution never closed: `unmatched "` inside double quotes, a parse
    /// error in an assignment.
    #[test]
    fn invalid_math_substitution_reparses_as_a_subshell() {
        assert_parity(r#"print "$(( a[ ))"; print rc=$?"#);
        assert_parity(r#"print "$(( a[1 ))"; print rc=$?"#);
        assert_parity(r#"x=$(( a[ )); print rc=$? "[$x]""#);
        // Control: the `((` command form already re-read correctly.
        assert_parity(r#"(( a[ )); print rc=$?"#);
    }

    /// The compiler must take the lexer's verdict. Unquoted, `$(( a[ ))`
    /// balances as TEXT, and compile_zsh's `strip_arith_subst` evaluated it as
    /// arithmetic (`0`) although the lexer had read a command substitution;
    /// `$(( a] ))` came out as "bad math expression" instead of running `a]`.
    #[test]
    fn unquoted_invalid_math_substitution_runs_the_subshell() {
        assert_parity(r#"print $(( a[ )); print rc=$?"#);
        assert_parity(r#"print $(( a[1 )); print rc=$?"#);
        assert_parity(r#"print $(( a] )); print rc=$?"#);
        // A `$` inside the failed scan is a two-byte token in the lexer
        // buffer; the rewind has to stop at the same BYTE length it started
        // from (c:Src/lex.c:523 `lexbuf.len`), or it eats the `(` and the
        // word's `$`: "parse error near `x=(((…'".
        assert_parity(r#"x=$(( $a ) ); print rc=$? "[$x]""#);
        assert_parity(r#"x=$(( $a[b )); print rc=$? "[$x]""#);
        assert_parity(r#"x="$(( $a[b ))"; print rc=$? "[$x]""#);
        assert_parity(r#"typeset x=$(( $a[b )); print rc=$?"#);
        // Controls: real arithmetic, and a substitution opening with a subshell.
        assert_parity(r#"print $(( 2 * 3 )) $((7/2))"#);
        assert_parity(r#"f(){ print "!$1!" }; print $((f a); f b)"#);
    }

    /// An unterminated `((` is a lexer error whose message names what the
    /// math scan collected (c:Src/lex.c:788-791 leaves `tokstr` on the lexer
    /// buffer): zsh says "parse error near ` 1 +'"; zshrs named the wrong
    /// token (`+`, `(`) because the scan never reported running out of input.
    #[test]
    fn unterminated_double_paren_reports_the_math_text() {
        if !zsh_available() {
            return;
        }
        for script in ["(( 1 +", "(("] {
            let z = Command::new(zsh_path()).args(["-fc", script]).output().expect("zsh");
            let r = Command::new(zshrs_bin())
                .args(["--zsh", "-f", "-c", script])
                .env_remove("ZSHRS_CACHE")
                .stdin(std::process::Stdio::null())
                .output()
                .expect("zshrs");
            assert_eq!(
                (String::from_utf8_lossy(&z.stderr), z.status.code()),
                (String::from_utf8_lossy(&r.stderr), r.status.code()),
                "stderr/status divergence on {script:?}"
            );
        }
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

/// c:Src/lex.c:2155-2292 — skipcomm parses the body of `$( … )` with
/// `parse_event(OUTPAR)` (c:2236) and keeps the raw text as the word. The
/// parser, not a paren counter, decides where the body ends, so a `)` that
/// is a case pattern, part of `${…}`, quoted, escaped, commented out or in a
/// here-document does not close it, and a body that does not parse is a
/// syntax error of the enclosing command (c:2244-2245 `lexstop = 1`).
mod body_is_parsed {
    use super::*;

    #[test]
    fn a_body_that_does_not_parse_is_a_syntax_error() {
        assert_parity("print start; echo $(|||) bar; print end");
        assert_parity("print start; echo $(a;;b) x; print end");
        assert_parity("print start; echo $(echo }) y; print end");
    }

    #[test]
    fn a_close_paren_the_parser_owns_does_not_end_the_body() {
        assert_parity("echo $(case x in x) echo c;; esac) y");
        assert_parity("echo $(case x in (x) echo c;; esac) y");
        assert_parity("echo $(echo ${x:-a)b}) z");
        assert_parity("echo $(echo \"(\" ) q; echo $(echo \\)) r; echo $(echo 'a)b') aa");
        assert_parity("echo $( # c )\necho hi) s");
        assert_parity("echo $(cat <<EOF\na ) b\nEOF\n) w");
        assert_parity("echo $(cat <<\\EOF\n$x ) b\nEOF\n) w");
    }

    #[test]
    fn separators_and_nesting_inside_the_body() {
        assert_parity("echo $(echo a; ) x; echo $(<<<x cat) bb; echo $(()) cc");
        assert_parity("echo $(echo $(echo n)) dd; echo $(echo `echo bq`) ee");
        assert_parity(r#"x=$(print -r -- "$(echo "in ) q")"); print -r -- $x"#);
        assert_parity("cat <(echo p) =(echo e) 2>/dev/null | head -1");
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

/// Nofork `${ cmd }` trailing-newline trim. The release zsh binary the
/// parity helpers use predates nofork substitution, so these pin the output
/// of the zsh dev tree (D10nofork.ztst). c:Src/subst.c:1908 `int trim =
/// (!EMULATION(EMULATE_ZSH)) ? 2 : !qt;` and c:2064-2069: unquoted strips ONE
/// newline, double-quoted strips NONE. The capture used to strip them all,
/// and a segment of a larger quoted word was taken as unquoted.
mod nofork_trim_zshrs_pin {
    use super::*;

    fn out(s: &str) -> String {
        run_zshrs(s).stdout
    }

    #[test]
    fn unquoted_strips_exactly_one_newline() {
        assert_eq!(out(r#"print -r -- ${ echo $'a\n\n\n' }."#), "a\n\n\n.\n");
        assert_eq!(out(r#"x=${ print a }; typeset -p x"#), "typeset x=a\n");
        assert_eq!(out(r#"x=a${ print b }c; typeset -p x"#), "typeset x=abc\n");
        assert_eq!(out(r#"w=${ print a } typeset -p w"#), "typeset w=a\n");
    }

    #[test]
    fn quoted_strips_nothing() {
        assert_eq!(out(r#"print -r -- "${ echo $'a\n\n' }""#), "a\n\n\n\n");
        assert_eq!(out(r#"print -r -- "${ print INNER } $?""#), "INNER\n 0\n");
        assert_eq!(out(r#"print -r -- "a${ print b }c""#), "ab\nc\n");
        assert_eq!(out(r#"x="${ print a }"; typeset -p x"#), "typeset x=$'a\\n'\n");
    }
}

/// Nofork substitution runs in the CURRENT shell (dev-tree zsh pins, see
/// `nofork_trim_zshrs_pin`). c:Src/subst.c:2094-2103: an `exit` in the body
/// ends the shell before the command around it runs, and c:2056 leaves the
/// body's errflag set. The capture restored the parent's exit/errflag state
/// the way `$( … )` must, so the command ran and the exit was lost.
mod nofork_current_shell_zshrs_pin {
    use super::*;

    #[test]
    fn exit_in_the_body_ends_the_shell_first() {
        for body in ["${ print x; exit 7 }", "${| REPLY=x; exit 7 }", "${(U)${ exit 7 }}"] {
            let r = run_zshrs(&format!("print A {body} B; print C"));
            assert_eq!((r.stdout.as_str(), r.exit), ("", 7), "{body}");
            let r = run_zshrs(&format!("(print A {body} B; print C); print rc=$?"));
            assert_eq!(r.stdout, "rc=7\n", "subshell: {body}");
        }
    }

    #[test]
    fn errexit_in_the_body_aborts_the_command() {
        let r = run_zshrs("(print A ${ setopt errexit; false; print no } B; print C)");
        assert_eq!((r.stdout.as_str(), r.exit), ("", 1));
    }

    /// c:Src/subst.c:183-186 — prefork removes an unquoted empty word.
    #[test]
    fn an_empty_unquoted_result_is_no_word() {
        assert_eq!(run_zshrs("print -l ${ true } ${| true } XX").stdout, "XX\n");
        assert_eq!(run_zshrs("a=(${ true }); print $#a").stdout, "0\n");
        assert_eq!(run_zshrs(r#"print -l "${ true }" XX"#).stdout, "\nXX\n");
        assert_eq!(run_zshrs("x=${ true }; print ${+x}").stdout, "1\n");
    }
}

/// Nofork `${|…}` / `${ … }` inside DOUBLE quotes (dev-tree zsh pins, see
/// `nofork_trim_zshrs_pin`). c:Src/lex.c:1631-1640 opens the substitution
/// as a command body in dquote_parse; c:1558-1576 tokenize a `{` in the
/// body as unquoted, so the body's own brace group does not close the `${`.
/// The body used to be scanned as a quoted string: a parse error.
mod nofork_in_double_quotes_zshrs_pin {
    use super::*;

    #[test]
    fn body_is_parsed_as_commands() {
        assert_eq!(run_zshrs(r#"print "A${| g() { print "q" ;} }B"; g"#).stdout, "AB\nq\n");
        assert_eq!(run_zshrs(r#"print "a${ { echo "}" ; } }b""#).stdout, "a}\nb\n");
        assert_eq!(run_zshrs(r#"print "${| REPLY=r }${ { echo b; } }""#).stdout, "rb\n\n");
        assert_eq!(run_zshrs(r#"x=3; print "${x}${| REPLY="{"}z""#).stdout, "3{z\n");
    }
}
