//! A QUOTED character in the pattern operand of `${var#pat}` / `${var%pat}` /
//! `${var/pat/rep}` (and their doubled, anchored, `:/` and `:#` forms) is a
//! literal character, not pattern syntax.
//!
//! C re-lexes the operand before matching:
//!
//! ```c
//! /* Src/subst.c:3371-3387 */
//! case '%':
//! case '#':
//! case Pound:
//! case '/':
//!     ...
//!     haserr = parse_subst_string(s);
//! ```
//!
//! `parse_subst_string` (Src/lex.c:1796) untokenizes the text and runs the
//! lexer over it again, so `"("` / `'('` come back as a quoted, plain `(` while
//! a bare `(` becomes the Inpar token; only the token is grouping syntax to
//! `patcompile`.
//!
//! zshrs folds the `${…}` body's lexer tokens back to plain characters before
//! its operator dispatch and used to drop the Snull/Dnull quote markers in that
//! fold, leaving a quoted `(` byte-identical to an unquoted one. The pattern
//! pre-tokenizer then made it Inpar:
//!
//! ```text
//! x="a(b"; print ${x//"("/Q}
//!   zsh   -> aQb
//!   zshrs -> zsh:1: bad pattern: (
//! x="a*b"; print ${x//"*"/Q}
//!   zsh   -> aQb
//!   zshrs -> Q
//! ```
//!
//! Backslash quoting (`\(`) and splices (`$y`, `"$y"`) were unaffected; the
//! rows for them pin that they stay that way.
//!
//! The second half pins a related user-visible failure on the same startup:
//! a `\'` inside a `$'…'` that is not the first thing in its word closed the
//! string early (the lexer spells that quote Bnull + `'`, and the `$'…'` scan
//! only honoured a raw backslash), so the rest of the string was expanded as
//! live text. powerlevel10k's instant-prompt file assigns
//! `__p9k_instant_prompt_param_sig=$'…\'\'…${${CONDA_PROMPT_MODIFIER#\\(}…}…'`,
//! which printed `bad pattern: \\(` at every interactive startup.
//!
//! stdout, "any stderr at all" and exit status are compared against the
//! reference `zsh`; tests no-op silently when `zsh` isn't on PATH.

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

/// The variables the rows use; removed from both environments so an inherited
/// value cannot fake (or hide) a divergence.
const ROW_VARS: &[&str] = &["qpo_x", "qpo_y", "qpo_v", "qpo_a", "qpo_u", "qpo_p"];

fn run(bin: &Path, args: &[&str], script: &str) -> std::process::Output {
    let mut cmd = Command::new(bin);
    cmd.args(args).arg(script).env_remove("ZSHRS_CACHE");
    for v in ROW_VARS {
        cmd.env_remove(v);
    }
    cmd.output().expect("invoke shell")
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = run(Path::new(zsh_path()), &["-fc"], script);
    let r = run(&zshrs_bin(), &["--zsh", "-f", "-c"], script);

    let z_out = String::from_utf8_lossy(&z.stdout).into_owned();
    let r_out = String::from_utf8_lossy(&r.stdout).into_owned();
    assert_eq!(
        z_out, r_out,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{z_out:?}\n--- zshrs ---\n{r_out:?}"
    );
    // The bug's loudest form was a `bad pattern:` diagnostic with an empty
    // stdout; the shells word their prefixes differently, so compare only
    // whether either produced one.
    assert_eq!(
        !z.stderr.is_empty(),
        !r.stderr.is_empty(),
        "stderr divergence on script:\n{script}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        String::from_utf8_lossy(&z.stderr),
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(
        z.status.code().unwrap_or(-1),
        r.status.code().unwrap_or(-1),
        "exit divergence on script:\n{script}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. The reported shapes and their controls.
// ═══════════════════════════════════════════════════════════════════════════

mod reported_shapes {
    use super::assert_parity;

    /// Exact text of the user's diagnostic: a quoted backslash followed by a
    /// quoted `(`.
    #[test]
    fn dquoted_backslash_then_paren() {
        assert_parity(r#"qpo_x='a\(b'; print -r -- ${qpo_x//"\\("/Q}"#);
    }

    #[test]
    fn escaped_backslash_then_dquoted_paren() {
        assert_parity(r#"qpo_x='a\(b'; print -r -- ${qpo_x//\\"("/Q}"#);
    }

    #[test]
    fn dquoted_paren() {
        assert_parity(r#"qpo_x='a(b'; print -r -- ${qpo_x//"("/Q}"#);
    }

    #[test]
    fn squoted_paren() {
        assert_parity(r#"qpo_x='a(b'; print -r -- ${qpo_x//'('/Q}"#);
    }

    /// Controls — these already worked and must keep working.
    #[test]
    fn backslash_escaped_paren() {
        assert_parity(r#"qpo_x='a(b'; print -r -- ${qpo_x//\(/Q}"#);
    }

    #[test]
    fn spliced_paren_bare_and_quoted() {
        assert_parity(r#"qpo_x='a(b'; qpo_y='('; print -r -- ${qpo_x//$qpo_y/Q} ${qpo_x//"$qpo_y"/Q}"#);
    }

    /// An unquoted `*` is still a pattern.
    #[test]
    fn unquoted_star_still_matches_everything() {
        assert_parity(r#"qpo_x='a*b'; print -r -- ${qpo_x//*/Q} ${qpo_x#a*} ${qpo_x%*b}"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Every operator shares the operand path.
// ═══════════════════════════════════════════════════════════════════════════

mod every_operator {
    use super::assert_parity;

    #[test]
    fn strip_operators() {
        assert_parity(
            r#"qpo_x='a(b'; print -r -- ${qpo_x#"a("} ${qpo_x##"a("} ${qpo_x%"(b"} ${qpo_x%%"(b"}"#,
        );
    }

    #[test]
    fn replace_operators() {
        assert_parity(
            r#"qpo_x='a*b'; print -r -- ${qpo_x/"*"/Q} ${qpo_x//'*'/Q} ${qpo_x/#"a*"/Q} ${qpo_x/%"*b"/Q} ${qpo_x:/"a*b"/Q}"#,
        );
    }

    #[test]
    fn element_filter_scalar_and_array() {
        assert_parity(r#"qpo_x='a(b'; print -r -- ${qpo_x:#"a(b"} end"#);
        assert_parity(r#"qpo_a=('a(b' c); print -r -- ${qpo_a:#"a(b"} ${qpo_a//"("/Q} ${qpo_a#"a("}"#);
    }

    /// Inside a double-quoted word the `'` reaches the operand as a plain
    /// byte; C's re-lex still reads it as quoting.
    #[test]
    fn inside_a_double_quoted_word() {
        assert_parity(r#"qpo_x='a(b'; print -r -- "${qpo_x//"("/Q}" "${qpo_x//'('/Q}" "${qpo_x#'a('}""#);
        assert_parity(r#"qpo_x='a*b'; print -r -- "${qpo_x//"*"/Q}" "${qpo_x//\*/Q}" "${qpo_x//*/Q}""#);
    }

    #[test]
    fn positional_parameters() {
        assert_parity(r#"set -- 'a(b' 'c*'; print -r -- "${@//"("/Q}" "${@//"*"/Q}""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Each metacharacter, quoted, is literal.
// ═══════════════════════════════════════════════════════════════════════════

mod each_metacharacter {
    use super::assert_parity;

    #[test]
    fn question_mark() {
        assert_parity(r#"qpo_x='a?b'; print -r -- ${qpo_x//"?"/Q} ${qpo_x//?/Q}"#);
    }

    #[test]
    fn brackets() {
        assert_parity(r#"qpo_x='a[b]'; print -r -- ${qpo_x//"[b]"/Q} ${qpo_x//'['/Q} ${qpo_x//[b]/Q}"#);
    }

    /// A quoted `#` / `%` at the start of a `//` pattern is not an anchor.
    #[test]
    fn anchor_characters() {
        assert_parity(r##"qpo_x='a#b'; print -r -- ${qpo_x//"#"/Q} ${qpo_x#"a#"}"##);
        assert_parity(r#"qpo_x='a%b'; print -r -- ${qpo_x//"%"/Q} ${qpo_x#"a%"} ${qpo_x//%b/Q}"#);
    }

    #[test]
    fn extendedglob_operators() {
        assert_parity(r#"setopt extendedglob; qpo_x='a~b'; print -r -- ${qpo_x//"~"/Q} ${qpo_x//"a~"/Q}"#);
        assert_parity(r#"setopt extendedglob; qpo_x='a^b'; print -r -- ${qpo_x//"^"/Q} ${qpo_x#"a^"}"#);
    }

    #[test]
    fn numeric_range() {
        assert_parity(r#"qpo_x='a<1-2>b'; print -r -- ${qpo_x//"<1-2>"/Q} ${qpo_x//'<'/Q}"#);
    }

    /// A quoted `!` inside a class is a member, not negation.
    #[test]
    fn bang_inside_a_class() {
        assert_parity(r#"qpo_x='a!b'; print -r -- ${qpo_x//["!"a]/Q} ${qpo_x//[!a]/Q}"#);
    }

    #[test]
    fn backslash() {
        assert_parity(r#"qpo_x='a\b'; print -r -- ${qpo_x//\\/Q} ${qpo_x//"\\"/Q} ${qpo_x//'\'/Q}"#);
        assert_parity(r#"qpo_x='a\(b'; print -r -- ${qpo_x//"\("/Q} ${qpo_x#'a\('}"#);
    }

    /// The decoded text of `$'…'` is literal too.
    #[test]
    fn dollar_single_quote() {
        assert_parity(r#"qpo_x='a*b'; print -r -- ${qpo_x//$'*'/Q} ${qpo_x#$'a*'} ${qpo_x//$'\x2a'/Q}"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Quote semantics: `$` inside `"…"` expands, inside `'…'` does not.
// ═══════════════════════════════════════════════════════════════════════════

mod quote_semantics {
    use super::assert_parity;

    #[test]
    fn single_quotes_do_not_expand() {
        assert_parity(r#"qpo_x='a$qpo_vb'; qpo_v=b; print -r -- ${qpo_x//'$qpo_v'/Q} ${qpo_x#'a$qpo_v'}"#);
    }

    /// A backslash-escaped `$` in a `//` pattern was expanded too.
    #[test]
    fn escaped_dollar_does_not_expand() {
        assert_parity(r#"qpo_x='a$qpo_vb'; qpo_v=b; print -r -- ${qpo_x//\$qpo_v/Q} ${qpo_x#a\$qpo_v}"#);
    }

    /// `"*$v"`: the `$v` expands, the `*` stays literal.
    #[test]
    fn double_quotes_expand_but_keep_metas_literal() {
        assert_parity(r#"qpo_x='a*bc'; qpo_v=b; print -r -- ${qpo_x//"*$qpo_v"/Q} ${qpo_x#"a*"}"#);
    }

    /// The closing quote ends the parameter name: `"$v"b` is `${v}b`.
    #[test]
    fn quote_ends_the_name() {
        assert_parity(r#"qpo_x=abc; qpo_v=a; print -r -- ${qpo_x#"$qpo_v"b} ${qpo_x#"$qpo_u"a} ${qpo_x#$qpo_u''a}"#);
    }

    /// `$name[…]` and `$name:mod` inside `"…"` still subscript / modify.
    #[test]
    fn subscript_and_modifier_inside_double_quotes() {
        assert_parity(r#"qpo_a=(x y); qpo_x=axb; print -r -- ${qpo_x#"a$qpo_a[1]"} ${qpo_x//b/"$qpo_a[2]"}"#);
        assert_parity(r#"qpo_p=/p/q; qpo_x=/p/r; print -r -- ${qpo_x#"$qpo_p:h"/}"#);
    }

    /// The replacement honours single quotes as well.
    #[test]
    fn replacement_single_quotes_do_not_expand() {
        assert_parity(r#"qpo_x=abc; qpo_v=Z; print -r -- ${qpo_x//b/'$qpo_v'} ${qpo_x//b/"$qpo_v"} ${qpo_x//b/"*"}"#);
    }

    #[test]
    fn default_word_single_quotes_do_not_expand() {
        assert_parity(r#"qpo_v=Z; print -r -- ${qpo_u:-'$qpo_v'} ${qpo_u:-"$qpo_v"} ${qpo_v:+'$qpo_v'}"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. GLOB_SUBST activates spliced values, never quoted or escaped text.
// ═══════════════════════════════════════════════════════════════════════════

mod globsubst {
    use super::assert_parity;

    #[test]
    fn quoted_and_escaped_paren() {
        assert_parity(r#"setopt globsubst; qpo_x='a(b'; print -r -- ${qpo_x#a\(} ${qpo_x#"a("} ${qpo_x//"("/Q}"#);
    }

    #[test]
    fn quoted_and_escaped_star() {
        assert_parity(r#"setopt globsubst; qpo_x='a*b'; print -r -- ${qpo_x#a\*} ${qpo_x#"a*"} ${qpo_x//"*"/Q}"#);
    }

    /// A spliced value still becomes a live pattern.
    #[test]
    fn splice_is_still_active() {
        assert_parity(r#"setopt globsubst; qpo_x=abc; qpo_v='*'; print -r -- ${qpo_x//$qpo_v/Q}"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. `\'` inside a `$'…'` that is not the start of its word.
// ═══════════════════════════════════════════════════════════════════════════

mod escaped_quote_in_dollar_single_quote {
    use super::assert_parity;

    #[test]
    fn assignment_value() {
        assert_parity(r#"qpo_v=Z; qpo_x=$'\'${qpo_v}'; print -r -- $qpo_x"#);
        assert_parity(r#"qpo_v=Z; typeset qpo_x=$'\'\'$qpo_v'; print -r -- $qpo_x"#);
    }

    #[test]
    fn after_a_word_prefix() {
        assert_parity(r#"qpo_v=Z; print -r -- x$'a\'b$qpo_v'"#);
    }

    /// A quoted backslash must not start an escape of its own.
    #[test]
    fn quoted_backslash_after_a_word_prefix() {
        assert_parity(r#"print -r -- x$'\\u0041' x$'a\\\\b'"#);
    }

    /// The powerlevel10k instant-prompt shape that printed
    /// `bad pattern: \\(` at every startup.
    #[test]
    fn p10k_instant_prompt_param_sig() {
        assert_parity(
            r#"typeset -g qpo_x=$'1\C-A\'\'\C-A${${${${qpo_u#\\(}% }%\\)}:-${qpo_p:t}}\C-A2'; print -r -- ${(V)qpo_x}"#,
        );
    }
}
