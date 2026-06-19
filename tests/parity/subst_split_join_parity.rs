//! Param-expansion split/join + case flags parity:
//! `${(s./.)V}` split-on-string, `${(j.x.)A}` join-with-string,
//! `${(w)V}` word-count, `${(C)V}` capitalize words,
//! `${(L)V}` lowercase, `${(U)V}` uppercase, `${(0)V}` NUL-split.

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

mod split {
    use super::*;

    /// `${(s./.)PATH}` split on /. Fixed: paramsubst's split + auto-
    /// splat now emits a Nularg sentinel (`\u{a1}`) for empty
    /// elements per c:Src/subst.c:36 `nulstring[]`, and prefork's
    /// remnulargs at c:Src/subst.c:170 strips it back to empty AFTER
    /// the empty-delete branch has already passed. Without that
    /// sentinel, the leading empty element of "/usr/local/bin" got
    /// deleted by prefork's `else if (!keep) uremnode` arm.
    #[test]
    fn split_on_slash() {
        assert_parity(r#"X="/usr/local/bin"; print -l "${(@s./.)X}""#);
    }

    /// `${(s.,.)X}` split on comma.
    #[test]
    fn split_on_comma() {
        assert_parity(r#"X="a,b,c,d"; print -l "${(@s.,.)X}""#);
    }

    /// `${(s.:.)X}` colon-delimited.
    #[test]
    fn split_on_colon() {
        assert_parity(r#"X="a:b:c"; print -l "${(@s.:.)X}""#);
    }

    /// Multi-char delimiter `${(s.,,.)X}`.
    #[test]
    fn split_on_multi_char_delimiter() {
        assert_parity(r#"X="a,,b,,c"; print -l "${(@s.,,.)X}""#);
    }

    /// Empty intermediate fields kept. Same Nularg fix as
    /// `split_on_slash`.
    #[test]
    fn split_keeps_empty_fields() {
        assert_parity(r#"X="a,,b"; print -l "${(@s.,.)X}""#);
    }
}

mod ps_split {
    use super::*;

    /// `${(ps.x.)X}` is split flag that supports special chars.
    /// Use `${(ps:\0:)X}` for NUL split.
    #[test]
    fn ps_split_on_nul_byte() {
        assert_parity(r#"X=$'a\0b\0c'; print -l "${(@ps:\0:)X}""#);
    }
}

mod join {
    use super::*;

    /// `${(j./.)A}` joins array with /.
    #[test]
    fn join_with_slash() {
        assert_parity(r#"arr=(usr local bin); echo "${(j./.)arr}""#);
    }

    /// Join with empty string concatenates.
    #[test]
    fn join_with_empty_string() {
        assert_parity(r#"arr=(a b c); echo "${(j..)arr}""#);
    }

    /// Join with multi-char string.
    #[test]
    fn join_with_arrow_separator() {
        assert_parity(r#"arr=(one two three); echo "${(j./->/)arr}""#);
    }

    /// Join + newline = `${(F)A}` shortcut.
    #[test]
    fn join_F_newline() {
        assert_parity(r#"arr=(a b c); echo "${(F)arr}""#);
    }
}

mod word_count {
    use super::*;

    /// `${(w)X}` count words (split on whitespace).
    #[test]
    fn count_three_words() {
        assert_parity(r#"X="one two three"; echo "${#${(z)X}}""#);
    }

    /// Words preserve quoting via (z).
    #[test]
    fn z_split_respects_quotes() {
        assert_parity(r#"X='one "two three" four'; arr=("${(z)X}"); echo "${#arr}""#);
    }
}

mod case_flags {
    use super::*;

    /// `${(U)X}` uppercase.
    #[test]
    fn U_uppercase() {
        assert_parity(r#"X=hello; echo "${(U)X}""#);
    }

    /// `${(L)X}` lowercase.
    #[test]
    fn L_lowercase() {
        assert_parity(r#"X=HELLO; echo "${(L)X}""#);
    }

    /// `${(C)X}` capitalize each word.
    #[test]
    fn C_capitalize_words() {
        assert_parity(r#"X="hello world foo bar"; echo "${(C)X}""#);
    }

    /// `${(C)X}` on already-uppercase.
    #[test]
    fn C_on_uppercase_lowers_rest() {
        assert_parity(r#"X="HELLO WORLD"; echo "${(C)X}""#);
    }
}

mod array_iterate {
    use super::*;

    /// Iterate over split result.
    #[test]
    fn iterate_split_result() {
        assert_parity(
            r#"
X="a:b:c"
for w in "${(@s.:.)X}"; do
  echo "[$w]"
done
"#,
        );
    }

    /// Iterate over joined array.
    #[test]
    fn iterate_joined_string() {
        assert_parity(
            r#"
arr=(red green blue)
echo "${(j., .)arr}"
"#,
        );
    }
}

mod combined_flags {
    use super::*;

    /// `${(sj)X}` mixed flags don't make sense alone — combined with delim.
    /// Test (Us) uppercase + split: zsh applies left-to-right.
    #[test]
    fn upper_then_split() {
        assert_parity(r#"X="hi:there"; print -l "${(@s.:.U)X}""#);
    }

    /// `${(LF)A}` lowercase each elem then join with newline.
    #[test]
    fn lower_then_F_join() {
        assert_parity(r#"arr=(HELLO WORLD); echo "${(LF)arr}""#);
    }
}

mod p_string_subst {
    use super::*;

    /// `${(p)X}` enables prompt-style substitution. Test integration.
    #[test]
    fn p_flag_processes_escape() {
        assert_parity(r#"X=$'a\nb'; echo "[${(p)X}]""#);
    }
}

mod ps_join_combined {
    use super::*;

    /// `${(pj.\n.)A}` join with newline (escape-processed).
    #[test]
    fn pj_join_with_newline_escape() {
        assert_parity(r#"arr=(a b c); echo "${(pj.\n.)arr}""#);
    }
}

/// Bare `$=NAME` (forced IFS word-split) applies to positional and
/// special parameter names too, not just alpha idents
/// (c:Src/subst.c:2554). `$=1` previously leaked through as literal.
mod bare_split_flag_special_names {
    use super::*;

    #[test]
    fn positional_single_digit() {
        assert_parity(r#"1="a b c"; print -l $=1"#);
    }

    #[test]
    fn positional_multi_digit() {
        assert_parity(r#"12="a b"; print -l $=12"#);
    }

    #[test]
    fn star_splits_positionals() {
        assert_parity(r#"set -- "a b"; print -l $=*"#);
    }

    #[test]
    fn at_splits_each_positional() {
        assert_parity(r#"set -- "a b" "c d"; print -l $=@"#);
    }

    #[test]
    fn double_equals_disables_split_on_positional() {
        assert_parity(r#"1="x y"; print -l $==1"#);
    }

    /// Alpha-ident name still works (regression guard).
    #[test]
    fn alpha_name_unaffected() {
        assert_parity(r#"v="a b"; print -l $=v"#);
    }

    /// Unset special name → empty (regression guard).
    #[test]
    fn unset_name_empty() {
        assert_parity(r#"print $=notset; echo done"#);
    }
}
