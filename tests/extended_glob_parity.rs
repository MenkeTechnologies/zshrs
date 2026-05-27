//! EXTENDED_GLOB pattern parity — (#i), (#l), (#a), (#b), (#m),
//! `^pat` negation, `~pat` exclusion. Uses [[ str == ${~pat} ]] form.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") { return PathBuf::from(p); }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("debug").join("zshrs")
}
fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() { "/opt/homebrew/bin/zsh" }
    else if Path::new("/usr/local/bin/zsh").exists() { "/usr/local/bin/zsh" }
    else { "/bin/zsh" }
}
fn zsh_available() -> bool {
    Command::new(zsh_path()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}
struct R { stdout: String, exit: i32 }
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path()).args(["-fc", s]).output().expect("zsh");
    R { stdout: String::from_utf8_lossy(&o.stdout).into_owned(), exit: o.status.code().unwrap_or(-1) }
}
fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin()).args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE").output().expect("zshrs");
    R { stdout: String::from_utf8_lossy(&o.stdout).into_owned(), exit: o.status.code().unwrap_or(-1) }
}
fn assert_parity(s: &str) {
    if !zsh_available() { return; }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}", z.stdout, r.stdout);
    assert_eq!(z.exit, r.exit);
}

mod hash_i_case_insensitive {
    use super::*;

    #[test]
    fn hash_i_matches_uppercase_against_lowercase_pat() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "FOO" == (#i)foo ]]; echo $?"#);
    }

    #[test]
    fn hash_i_matches_lowercase_against_uppercase_pat() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo" == (#i)FOO ]]; echo $?"#);
    }

    #[test]
    fn hash_i_matches_mixed_case() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "FoO" == (#i)foo ]]; echo $?"#);
    }

    #[test]
    fn hash_i_rejects_different_letters() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "bar" == (#i)foo ]]; echo $?"#);
    }
}

mod hash_l_lowercase_pat_matches_uppercase_text {
    use super::*;

    /// `(#l)foo` — lowercase letter in pattern matches lowercase or
    /// uppercase in text (asymmetric: pattern→text only).
    #[test]
    fn hash_l_lowercase_pat_matches_uppercase_text() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "FOO" == (#l)foo ]]; echo $?"#);
    }

    #[test]
    fn hash_l_lowercase_pat_matches_lowercase_text() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo" == (#l)foo ]]; echo $?"#);
    }

    /// `(#l)FOO` — uppercase in pat REQUIRES uppercase in text.
    #[test]
    fn hash_l_uppercase_pat_requires_uppercase_text() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo" == (#l)FOO ]]; echo $?"#);
    }
}

mod hash_a_approximate_match {
    use super::*;

    /// `(#a1)` — 1 substitution distance allowed.
    #[test]
    fn hash_a1_one_substitution() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "fop" == (#a1)foo ]]; echo $?"#);
    }

    /// `(#a2)` — 2 substitutions.
    #[test]
    fn hash_a2_two_substitutions() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "fxy" == (#a2)foo ]]; echo $?"#);
    }

    /// `(#a1)` rejects 2 substitutions.
    #[test]
    fn hash_a1_rejects_two_substitutions() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "fxy" == (#a1)foo ]]; echo $?"#);
    }
}

mod hash_m_match_var {
    use super::*;

    /// `(#m)pat` populates $MATCH on success.
    #[test]
    #[ignore = "ZSHRS BUG: (#m) doesn't populate $MATCH"]
    fn hash_m_populates_match_var() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "hello" == (#m)he* ]] && echo "MATCH=$MATCH""#);
    }

    #[test]
    #[ignore = "ZSHRS BUG: (#m) doesn't populate $MATCH for any pattern"]
    fn hash_m_match_var_full_match() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "abc123" == (#m)*[0-9]* ]] && echo "MATCH=$MATCH""#);
    }
}

mod hash_b_backref {
    use super::*;

    /// `(#b)pat` populates $match[] array with capture groups.
    #[test]
    #[ignore = "ZSHRS BUG: (#b) doesn't populate $match[] array"]
    fn hash_b_captures_into_match_array() {
        assert_parity(r#"
setopt EXTENDED_GLOB
[[ "hello-world" == (#b)(*)-(*) ]] && echo "1=${match[1]} 2=${match[2]}"
"#);
    }

    #[test]
    #[ignore = "ZSHRS BUG: (#b) doesn't populate $match[] for three captures"]
    fn hash_b_three_captures() {
        assert_parity(r#"
setopt EXTENDED_GLOB
[[ "a:b:c" == (#b)(*):(*):(*) ]] && echo "${match[1]}|${match[2]}|${match[3]}"
"#);
    }
}

mod caret_negation {
    use super::*;

    /// `^pat` — matches anything NOT matching pat (zsh ext-glob).
    #[test]
    #[ignore = "ZSHRS BUG: ^pat negation doesn't match non-matching strings"]
    fn caret_negation_matches_non_matching() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "bar" == ^foo ]]; echo $?"#);
    }

    #[test]
    fn caret_negation_rejects_matching() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo" == ^foo ]]; echo $?"#);
    }
}

mod tilde_exclusion {
    use super::*;

    /// `pat~excl` — match pat but exclude excl.
    /// e.g. `*~*.bak` = all files except those ending in `.bak`.
    #[test]
    #[ignore = "ZSHRS BUG: pat~excl exclusion form doesn't accept non-excluded matches"]
    fn tilde_excludes_subset() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo.txt" == *~*.bak ]]; echo $?"#);
    }

    #[test]
    fn tilde_exclusion_rejects_excluded() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo.bak" == *~*.bak ]]; echo $?"#);
    }
}

mod alternation {
    use super::*;

    /// `(a|b|c)` — alternation (works even without EXTENDED_GLOB).
    #[test]
    fn alternation_matches_first() {
        assert_parity(r#"[[ "foo" == (foo|bar|baz) ]]; echo $?"#);
    }

    #[test]
    fn alternation_matches_middle() {
        assert_parity(r#"[[ "bar" == (foo|bar|baz) ]]; echo $?"#);
    }

    #[test]
    fn alternation_rejects_outside() {
        assert_parity(r#"[[ "qux" == (foo|bar|baz) ]]; echo $?"#);
    }
}

mod combined {
    use super::*;

    /// `(#i)(foo|bar)` case-insensitive alternation.
    #[test]
    fn hash_i_with_alternation() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "FOO" == (#i)(foo|bar) ]]; echo $?"#);
    }

    /// `(#b)(*).log` capture with extension match.
    #[test]
    #[ignore = "ZSHRS BUG: (#b) capture in combined form doesn't populate $match[]"]
    fn hash_b_with_extension_match() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "access.log" == (#b)(*).log ]] && echo "name=${match[1]}""#);
    }
}

mod number_qualifier_quantifier {
    use super::*;

    /// `x##` — one or more x's.
    #[test]
    fn double_hash_one_or_more() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "aaa" == a## ]]; echo $?"#);
    }

    /// `x##` rejects zero.
    #[test]
    fn double_hash_rejects_zero() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "" == a## ]]; echo $?"#);
    }

    /// `x#` — zero or more.
    #[test]
    fn single_hash_zero_or_more() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "" == a# ]]; echo $?"#);
    }
}
