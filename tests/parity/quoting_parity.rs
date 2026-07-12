//! Quoting parity tests — single quotes, double quotes, ANSI-C `$'…'`,
//! backslash escapes.

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

mod single_quotes {
    use super::*;

    #[test]
    fn single_quote_literal() {
        assert_parity(r#"echo 'hello world'"#);
    }

    #[test]
    fn single_quote_no_var_expansion() {
        assert_parity(r#"X=value; echo '$X'"#);
    }

    #[test]
    fn single_quote_no_backslash_escape() {
        assert_parity(r#"echo 'a\nb'"#);
    }

    #[test]
    fn single_quote_preserves_double_quote() {
        assert_parity(r#"echo 'hi "there"'"#);
    }

    #[test]
    fn single_quote_empty() {
        assert_parity(r#"echo ''"#);
    }

    #[test]
    fn single_quote_with_spaces() {
        assert_parity(r#"echo '  multi  space  '"#);
    }
}

mod double_quotes {
    use super::*;

    #[test]
    fn double_quote_literal() {
        assert_parity(r#"echo "hello world""#);
    }

    #[test]
    fn double_quote_expands_dollar_var() {
        assert_parity(r#"X=value; echo "$X""#);
    }

    #[test]
    fn double_quote_expands_braced_var() {
        assert_parity(r#"X=value; echo "${X}_more""#);
    }

    #[test]
    fn double_quote_expands_command_subst() {
        assert_parity(r#"echo "now: $(echo current)""#);
    }

    #[test]
    fn double_quote_expands_arith() {
        assert_parity(r#"echo "sum: $((2+3))""#);
    }

    #[test]
    fn double_quote_preserves_single_quote() {
        assert_parity(r#"echo "don't""#);
    }

    #[test]
    fn double_quote_backslash_escapes_dollar() {
        assert_parity(r#"X=value; echo "\$X""#);
    }

    #[test]
    fn double_quote_backslash_escapes_double_quote() {
        assert_parity(r#"echo "say \"hi\"""#);
    }

    #[test]
    fn double_quote_backslash_escapes_backslash() {
        assert_parity(r#"echo "back\\slash""#);
    }

    /// In double quotes, `\n` is LITERAL (no escape processing).
    #[test]
    fn double_quote_backslash_n_is_literal() {
        assert_parity(r#"echo "a\nb""#);
    }
}

mod ansi_c_quote {
    use super::*;

    /// `$'\n'` → newline.
    #[test]
    fn ansi_quote_newline() {
        assert_parity(r#"echo $'a\nb'"#);
    }

    /// `$'\t'` → tab.
    #[test]
    fn ansi_quote_tab() {
        assert_parity(r#"echo $'a\tb'"#);
    }

    /// `$'\r'` → carriage return.
    #[test]
    fn ansi_quote_carriage_return() {
        assert_parity(r#"echo -n $'\r' | wc -c"#);
    }

    /// `$'\x41'` → 'A' (hex escape).
    #[test]
    fn ansi_quote_hex_escape() {
        assert_parity(r#"echo $'\x41'"#);
    }

    /// `$'\033'` → ESC (octal escape).
    #[test]
    fn ansi_quote_octal_escape_byte_count() {
        assert_parity(r#"echo -n $'\033' | wc -c"#);
    }

    /// `$'é'` → 'é' (Unicode escape).
    #[test]
    fn ansi_quote_unicode_escape() {
        assert_parity(r#"echo $'é'"#);
    }

    /// `$'\\'` → literal backslash.
    #[test]
    fn ansi_quote_backslash_literal() {
        assert_parity(r#"echo $'\\'"#);
    }

    /// `$''` → empty.
    #[test]
    fn ansi_quote_empty() {
        assert_parity(r#"echo "[$'']""#);
    }

    /// `$'single'` with no escapes — same as literal text.
    #[test]
    fn ansi_quote_no_escapes_passes_through() {
        assert_parity(r#"echo $'hello'"#);
    }
}

mod backslash_escape {
    use super::*;

    /// `\$` outside quotes — `$` literal.
    #[test]
    fn backslash_escapes_dollar_outside_quotes() {
        assert_parity(r#"X=value; echo \$X"#);
    }

    /// `\\` outside quotes — single backslash.
    #[test]
    fn backslash_backslash_outside_quotes() {
        assert_parity(r#"echo \\"#);
    }

    /// `\n` outside quotes — `n` literal (no escape processing).
    #[test]
    fn backslash_n_outside_quotes_is_literal_n() {
        assert_parity(r#"echo \n"#);
    }

    /// `\ ` outside quotes — escape the space, keep as single arg.
    #[test]
    fn backslash_space_outside_quotes() {
        assert_parity(r#"echo hi\ there"#);
    }
}

mod mixed_quoting {
    use super::*;

    /// Concatenation: `'literal'"$X"` produces concatenation.
    #[test]
    fn single_then_double_concatenate() {
        assert_parity(r#"X=world; echo 'hello '"$X""#);
    }

    /// Quote inside quote via concat.
    #[test]
    fn double_quote_concat_with_single_for_internal_quote() {
        assert_parity(r#"echo "say "'"hi"'" today""#);
    }

    /// ANSI-C followed by double for var expansion.
    #[test]
    fn ansi_then_double_concat() {
        assert_parity(r#"X=val; echo $'\t'"$X""#);
    }
}

mod empty_and_space {
    use super::*;

    /// `""` empty arg — counts as one positional.
    #[test]
    fn empty_double_quote_is_arg() {
        assert_parity(r#"f() { echo $#; }; f a "" b"#);
    }

    /// `''` empty arg — same.
    #[test]
    fn empty_single_quote_is_arg() {
        assert_parity(r#"f() { echo $#; }; f a '' b"#);
    }

    /// Quoted space stays one arg.
    #[test]
    fn quoted_space_stays_one_arg() {
        assert_parity(r#"f() { echo $#; }; f a "b c" d"#);
    }

    /// Unquoted space splits — three args.
    #[test]
    fn unquoted_space_splits() {
        assert_parity(r#"f() { echo $#; }; X="b c"; f a $X d"#);
    }

    /// Quoted var preserves whitespace.
    #[test]
    fn quoted_var_preserves_whitespace() {
        assert_parity(r#"f() { echo $#; }; X="b c"; f a "$X" d"#);
    }
}

/// Single-quoted `${...[...]}` fragments concatenated with an unquoted
/// expansion must stay LITERAL — the `${`, `[`, `]`, `}` inside single
/// quotes are ordinary chars, so the result is string concatenation, NOT
/// a live subscripted / flagged parameter substitution.
///
/// Regression: zshrs's `${NAME[KEY]}` / `${(flags)NAME[KEY]}` compile
/// fast paths matched on `untokenize(s)`, which strips the single-quote
/// (Snull) markers, so `'${foo['$((1+1))']}'` was misread as the live
/// access `${foo[2]}` and evaluated at build time. powerlevel10k builds
/// its deferred prompt from exactly this shape
/// (`_p9k_prompt_prefix_left='${(e)_p9k_t['$idx']}'`,
/// internal/p10k.zsh:8490) — evaluating it early dropped every prompt
/// segment, rendering an empty prompt.
mod sq_literal_brace_not_live {
    use super::*;

    /// Plain subscript split across single-quoted fragments stays literal.
    #[test]
    fn sq_subscript_stays_literal() {
        assert_parity(r#"foo=(a b c d); x='${foo['$((1+1))']}'; print -r -- "$x""#);
    }

    /// `(e)`-flagged subscript split across single quotes stays literal.
    #[test]
    fn sq_eval_flag_subscript_stays_literal() {
        assert_parity(r#"foo=(a b c d); x='${(e)foo['$((1+1))']}'; print -r -- "$x""#);
    }

    /// Literal (non-arith) middle fragment also stays literal.
    #[test]
    fn sq_literal_middle_stays_literal() {
        assert_parity(r#"x='${(e)foo['X']}'; print -r -- "$x""#);
    }

    /// The p10k deferred-template shape: `${(e)arr[$#arr - k]}`.
    #[test]
    fn sq_p10k_deferred_template_shape() {
        assert_parity(
            r#"typeset -a t=(a b c d e f g); integer k=0; x='${(e)t['$(($#t - k))']}'; print -r -- "$x""#,
        );
    }

    /// A genuinely LIVE unquoted subscript must still evaluate (no regression).
    #[test]
    fn live_subscript_still_evaluates() {
        assert_parity(r#"foo=(p q r); k=2; print -r -- "${foo[$k]}""#);
    }

    /// A genuinely LIVE unquoted flagged subscript must still evaluate.
    #[test]
    fn live_flag_subscript_still_evaluates() {
        assert_parity(r#"foo=(p q r); print -r -- "${(U)foo[1]}""#);
    }

    /// A genuinely LIVE assoc access must still evaluate.
    #[test]
    fn live_assoc_still_evaluates() {
        assert_parity(r#"typeset -A m=(kk vv); print -r -- "${m[kk]}""#);
    }
}
