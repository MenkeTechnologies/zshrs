//! A backslash that a SPLICED value contributes to the pattern operand of
//! `${var#pat}` / `${var%pat}` / `${var/pat/rep}` is a literal backslash
//! CHARACTER, not an escape of the character after it.
//!
//! C never has to decide: the pattern operand is re-lexed before the splice
//!
//! ```c
//! /* Src/subst.c:3374-3387 */
//! case '%': case '#': case Pound: case '/':
//!     haserr = parse_subst_string(s);
//! /* Src/subst.c:3423 */
//!     singsub(&s);
//! ```
//!
//! so a SOURCE `\(` is already the pair `Bnull (` (Src/lex.c:1268
//! `add(Bnull)`) when `singsub` runs, and prefork's `remnulargs(getdata(node))`
//! (Src/subst.c:169, Src/glob.c:3673-3681) then strips the marker — leaving a
//! BARE `(`, which is literal because `patcompile` dispatches on TOKEN bytes
//! only (Src/pattern.c:248 `zpc_chars`). The splice arrives raw through
//! `strcatsub` (Src/subst.c:814-835), which only ever `shtokenize`s the value,
//! and then under GLOB_SUBST. So with GLOB_SUBST off every raw character of
//! the operand — backslash included — is literal.
//!
//! zshrs spelled the source escape as a raw `\X` pair instead, which made it
//! byte-identical to a spliced backslash after `singsub`, and the post-splice
//! pass honoured the spliced one as an escape. That is the silent
//! over-matching direction — zshrs replaced where zsh leaves the string alone:
//!
//! ```text
//! x='a(b'; y='('; print ${x//${(b)y}/Q}
//!   zsh   -> a(b      (the pattern is the two characters `\` `(`)
//!   zshrs -> aQb      (the `\` was read as quoting the `(`)
//! x='a(b'; y='('; print ${x//${(q)y}/Q}      same
//! x='a\(b'; y='\('; print ${x//$y/Q}
//!   zsh   -> aQb
//!   zshrs -> a\Qb     (only the `(` matched)
//! ```
//!
//! `(b)` and `(q)` exist precisely to make a value safe to splice as a
//! literal, so reading their backslash as an escape defeated their purpose.
//!
//! Under GLOB_SUBST the two spellings genuinely agree: `shtokenize` reads a
//! spliced `\X` as a quoted literal X too (Src/glob.c:3597-3605,
//! `s[-1] = Bnullkeep`). The GLOB_SUBST rows below pin that, and that a
//! backslash-escaped SOURCE metacharacter stays literal there.
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
const ROW_VARS: &[&str] = &["sbp_x", "sbp_y"];

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

/// Every operator that routes through the pattern operand, for one spliced
/// value against one subject. The `${x:#pat}` form is in the list too because
/// it shares the same `singsub` call.
fn assert_all_operators(subject: &str, value: &str, splice: &str) {
    for op in [
        format!("${{sbp_x//{splice}/Q}}"),
        format!("${{sbp_x/{splice}/Q}}"),
        format!("${{sbp_x#{splice}}}"),
        format!("${{sbp_x##{splice}}}"),
        format!("${{sbp_x%{splice}}}"),
        format!("${{sbp_x%%{splice}}}"),
        format!("${{sbp_x:#{splice}}}"),
    ] {
        assert_parity(&format!(
            "sbp_x={subject}; sbp_y={value}; print -r -- {op}"
        ));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. The reported shapes.
// ═══════════════════════════════════════════════════════════════════════════

mod reported_shapes {
    use super::assert_parity;

    /// `${(b)y}` expands to the two characters `\` `(`. Without `$~` that is
    /// the literal two-character pattern, which does not occur in `a(b`.
    #[test]
    fn b_flag_paren_does_not_match_bare_paren() {
        assert_parity(r#"sbp_x='a(b'; sbp_y='('; print -r -- ${sbp_x//${(b)sbp_y}/Q}"#);
    }

    #[test]
    fn q_flag_paren_does_not_match_bare_paren() {
        assert_parity(r#"sbp_x='a(b'; sbp_y='('; print -r -- ${sbp_x//${(q)sbp_y}/Q}"#);
    }

    /// The other half of the contract: it DOES match a subject that really
    /// holds backslash + paren.
    #[test]
    fn b_flag_paren_matches_backslash_paren() {
        assert_parity(r#"sbp_x='a\(b'; sbp_y='('; print -r -- ${sbp_x//${(b)sbp_y}/Q}"#);
    }

    /// A plain `$y` splice carrying a `\X` pair has the same rule — this one
    /// needed no flag to reproduce.
    #[test]
    fn plain_splice_of_backslash_paren() {
        assert_parity(r#"sbp_x='a(b'; sbp_y='\('; print -r -- ${sbp_x//$sbp_y/Q}"#);
        assert_parity(r#"sbp_x='a\(b'; sbp_y='\('; print -r -- ${sbp_x//$sbp_y/Q}"#);
    }

    /// `(b)`/`(q)` quote `*` the same way, and the over-match there replaced
    /// an ordinary character run.
    #[test]
    fn b_flag_star_does_not_match_bare_star() {
        assert_parity(r#"sbp_x='a*b'; sbp_y='*'; print -r -- ${sbp_x//${(b)sbp_y}/Q}"#);
        assert_parity(r#"sbp_x='a\*b'; sbp_y='*'; print -r -- ${sbp_x//${(b)sbp_y}/Q}"#);
    }

    /// A value that is nothing BUT a backslash, and one that ends in one —
    /// the shape that used to swallow the character after it.
    #[test]
    fn lone_and_trailing_backslash_values() {
        assert_parity(r#"sbp_x='a\b'; sbp_y='\'; print -r -- ${sbp_x//$sbp_y/Q}"#);
        assert_parity(r#"sbp_x='a\b'; sbp_y='a\'; print -r -- ${sbp_x//$sbp_y/Q}"#);
        assert_parity(r#"sbp_x='[a\b'; sbp_y='[a\'; print -r -- ${sbp_x//$sbp_y/Q}"#);
    }

    /// Controls — a SOURCE escape must still quote its character.
    #[test]
    fn source_escape_still_quotes() {
        assert_parity(r#"sbp_x='a(b'; print -r -- ${sbp_x//\(/Q}"#);
        assert_parity(r#"sbp_x='a*b'; print -r -- ${sbp_x//\*/Q}"#);
        assert_parity(r#"sbp_x='axb'; print -r -- ${sbp_x//\*/Q}"#);
    }

    /// Control — an unquoted spliced metacharacter is still literal without
    /// GLOB_SUBST, and `$~` still makes it a pattern.
    #[test]
    fn tilde_still_activates_the_splice() {
        assert_parity(r#"sbp_x='axb'; sbp_y='a*b'; print -r -- ${sbp_x//$sbp_y/Q}"#);
        assert_parity(r#"sbp_x='axb'; sbp_y='a*b'; print -r -- ${sbp_x//${~sbp_y}/Q}"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Every metacharacter `(b)` / `(q)` can quote, across every operator.
// ═══════════════════════════════════════════════════════════════════════════

mod every_metachar {
    use super::{assert_all_operators, assert_parity};

    /// `(b)` backslash-quotes each of these, so the spliced pattern is the
    /// two literal characters and must not match the bare metacharacter.
    #[test]
    fn b_flag_over_all_metachars_all_operators() {
        for ch in ['*', '?', '[', ']', '(', ')', '|', '#', '~', '\\'] {
            assert_all_operators(&format!("'a{ch}b'"), &format!("'{ch}'"), "${(b)sbp_y}");
        }
    }

    /// The same sweep with the `\X` pair written straight into the value, so
    /// no quoting flag is involved in producing the backslash.
    #[test]
    fn plain_splice_of_escape_pair_all_operators() {
        for ch in ['*', '?', '[', ']', '(', ')', '|', '#', '~'] {
            assert_all_operators(&format!("'a{ch}b'"), &format!(r"'\{ch}'"), "$sbp_y");
            assert_all_operators(&format!(r"'a\{ch}b'"), &format!(r"'\{ch}'"), "$sbp_y");
        }
    }

    /// Same corpus, with the value assigned per row so the splice is real.
    #[test]
    fn b_flag_mid_and_prefix_subjects() {
        for ch in ['*', '?', '[', ']', '(', ')', '|', '#', '~', '\\'] {
            for subject in [format!("a{ch}b"), format!("{ch}ab")] {
                assert_parity(&format!(
                    r#"sbp_x='{subject}'; sbp_y='{ch}'; print -r -- ${{sbp_x//${{(b)sbp_y}}/Q}} ${{sbp_x#${{(b)sbp_y}}}} ${{sbp_x%${{(b)sbp_y}}}}"#
                ));
            }
        }
    }

    #[test]
    fn q_flag_mid_and_prefix_subjects() {
        for ch in ['*', '?', '[', ']', '(', ')', '|', '#', '~', '\\'] {
            for subject in [format!("a{ch}b"), format!("{ch}ab")] {
                assert_parity(&format!(
                    r#"sbp_x='{subject}'; sbp_y='{ch}'; print -r -- ${{sbp_x//${{(q)sbp_y}}/Q}} ${{sbp_x#${{(q)sbp_y}}}} ${{sbp_x%${{(q)sbp_y}}}}"#
                ));
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. GLOB_SUBST and `$~` are unchanged: there a spliced `\X` IS a quoted X,
//    because C shtokenizes the value (Src/glob.c:3597-3605).
// ═══════════════════════════════════════════════════════════════════════════

mod globsubst_unchanged {
    use super::assert_parity;

    #[test]
    fn globsubst_spliced_escape_quotes_its_char() {
        assert_parity(
            r#"setopt globsubst; sbp_x='a(b'; sbp_y='('; print -r -- ${sbp_x//${(b)sbp_y}/Q}"#,
        );
        assert_parity(
            r#"setopt globsubst; sbp_x='a\(b'; sbp_y='('; print -r -- ${sbp_x//${(b)sbp_y}/Q}"#,
        );
    }

    #[test]
    fn globsubst_source_escape_stays_literal() {
        assert_parity(r#"setopt globsubst; sbp_x='a(b'; print -r -- ${sbp_x#a\(}"#);
        assert_parity(r#"setopt globsubst; sbp_x='a*b'; print -r -- ${sbp_x//\*/Q}"#);
        assert_parity(r#"setopt globsubst; sbp_x='axb'; print -r -- ${sbp_x//\*/Q}"#);
    }

    #[test]
    fn globsubst_bare_splice_is_a_pattern() {
        assert_parity(r#"setopt globsubst; sbp_x='axb'; sbp_y='a*b'; print -r -- ${sbp_x//$sbp_y/Q}"#);
    }

    /// A nested `${~…}` tokenizes only the value it splices; the enclosing
    /// expansion's own metacharacters stay as they were.
    #[test]
    fn nested_tilde_scoping() {
        assert_parity(r#"sbp_y=X; sbp_x='a|b'; print -r -- ${sbp_x%%${~sbp_y}*}"#);
        assert_parity(r#"sbp_x=a1b; sbp_y='[0-9]'; print -r -- ${sbp_x#*${~sbp_y}}"#);
    }
}
