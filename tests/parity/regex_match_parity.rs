//! `[[ string =~ regex ]]` regex-match operator parity tests.
//!
//! zsh uses ERE by default. `$MATCH` holds the matched string,
//! `$match` is array of capture groups, `$MBEGIN`/`$MEND` give offsets.

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

mod basic_match {
    use super::*;

    /// `[[ str =~ pat ]]` true → exit 0.
    #[test]
    fn match_basic_literal() {
        assert_parity(r#"[[ "hello" =~ "ell" ]]; echo $?"#);
    }

    /// No match → exit 1.
    #[test]
    fn no_match_exit_1() {
        assert_parity(r#"[[ "hello" =~ "xyz" ]]; echo $?"#);
    }

    /// Empty string vs anything → no match.
    #[test]
    fn empty_string_no_match() {
        assert_parity(r#"[[ "" =~ "x" ]]; echo $?"#);
    }

    /// Empty pattern matches anything.
    #[test]
    fn empty_pattern_matches() {
        assert_parity(r#"[[ "hello" =~ "" ]]; echo $?"#);
    }
}

mod metachars {
    use super::*;

    /// `^` start anchor.
    #[test]
    fn caret_start_anchor() {
        assert_parity(r#"[[ "foobar" =~ "^foo" ]]; echo $?"#);
    }

    #[test]
    fn caret_start_anchor_no_match() {
        assert_parity(r#"[[ "barfoo" =~ "^foo" ]]; echo $?"#);
    }

    /// `$` end anchor.
    #[test]
    fn dollar_end_anchor() {
        assert_parity(r#"[[ "foobar" =~ "bar$" ]]; echo $?"#);
    }

    /// `.` any char.
    #[test]
    fn dot_any_char() {
        assert_parity(r#"[[ "abc" =~ "a.c" ]]; echo $?"#);
    }

    /// `*` zero-or-more.
    #[test]
    fn star_quantifier() {
        assert_parity(r#"[[ "aaab" =~ "a*b" ]]; echo $?"#);
    }

    /// `+` one-or-more.
    #[test]
    fn plus_quantifier() {
        assert_parity(r#"[[ "aaab" =~ "a+b" ]]; echo $?"#);
    }

    /// `?` zero-or-one.
    #[test]
    fn question_quantifier() {
        assert_parity(r#"[[ "ab" =~ "a?b" ]]; echo $?"#);
    }

    /// `[abc]` character class.
    #[test]
    fn char_class() {
        assert_parity(r#"[[ "b" =~ "[abc]" ]]; echo $?"#);
    }

    /// `[^abc]` negated class.
    #[test]
    fn negated_char_class() {
        assert_parity(r#"[[ "z" =~ "[^abc]" ]]; echo $?"#);
    }
}

mod match_variable {
    use super::*;

    /// $MATCH = matched substring.
    #[test]
    fn match_var_set_after_success() {
        assert_parity(r#"[[ "hello world" =~ "world" ]] && echo "[$MATCH]""#);
    }

    /// $MATCH preserved across non-match (zsh: not modified).
    #[test]
    fn match_var_after_failed_match() {
        assert_parity(
            r#"
[[ "abc" =~ "abc" ]]
[[ "xyz" =~ "qqq" ]]
echo "[$MATCH]"
"#,
        );
    }
}

mod match_array {
    use super::*;

    /// $match[1] holds first capture group.
    #[test]
    fn match_array_first_capture() {
        assert_parity(
            r#"
[[ "hello123world" =~ "([a-z]+)([0-9]+)([a-z]+)" ]]
echo "${match[1]}/${match[2]}/${match[3]}"
"#,
        );
    }

    /// $#match = number of capture groups.
    #[test]
    fn match_array_count() {
        assert_parity(
            r#"
[[ "abc" =~ "(a)(b)(c)" ]]
echo "${#match}"
"#,
        );
    }
}

mod offsets {
    use super::*;

    /// $MBEGIN / $MEND give 1-indexed start/end of match.
    #[test]
    fn mbegin_mend_offsets() {
        assert_parity(
            r#"
[[ "foobar" =~ "ob" ]]
echo "$MBEGIN/$MEND"
"#,
        );
    }

    /// $mbegin[N] / $mend[N] for capture groups.
    #[test]
    fn mbegin_array_for_groups() {
        assert_parity(
            r#"
[[ "abcdef" =~ "(b)(d)" ]]
echo "${mbegin[1]}/${mend[1]}/${mbegin[2]}/${mend[2]}"
"#,
        );
    }
}

mod case_sensitivity {
    use super::*;

    /// Default case-sensitive.
    #[test]
    fn default_case_sensitive() {
        assert_parity(r#"[[ "HELLO" =~ "hello" ]]; echo $?"#);
    }

    /// `(?i)` case-insensitive inline flag (POSIX ERE w/o, depends on impl).
    /// Linux+macOS may differ.
    #[test]
    fn case_insensitive_match_lowercase() {
        assert_parity(r#"[[ "hello" =~ "hello" ]]; echo $?"#);
    }
}

mod variable_as_pattern {
    use super::*;

    /// Pattern from variable. zsh expands $PAT as a regex.
    #[test]
    fn pattern_in_variable() {
        assert_parity(r#"PAT="^[0-9]+$"; [[ "12345" =~ $PAT ]]; echo $?"#);
    }

    #[test]
    fn pattern_in_variable_no_match() {
        assert_parity(r#"PAT="^[0-9]+$"; [[ "abc" =~ $PAT ]]; echo $?"#);
    }
}

mod alternation {
    use super::*;

    /// `a|b` alternation.
    #[test]
    fn alternation_left_match() {
        assert_parity(r#"[[ "cat" =~ "cat|dog" ]]; echo $?"#);
    }

    #[test]
    fn alternation_right_match() {
        assert_parity(r#"[[ "dog" =~ "cat|dog" ]]; echo $?"#);
    }

    #[test]
    fn alternation_no_match() {
        assert_parity(r#"[[ "bird" =~ "cat|dog" ]]; echo $?"#);
    }
}

mod count_quantifiers {
    use super::*;

    /// `{N}` exactly N.
    #[test]
    fn brace_exact_count() {
        assert_parity(r#"[[ "aaaa" =~ "a{4}" ]]; echo $?"#);
    }

    #[test]
    fn brace_exact_count_too_few() {
        assert_parity(r#"[[ "aaa" =~ "^a{4}$" ]]; echo $?"#);
    }

    /// `{N,M}` between N and M.
    #[test]
    fn brace_range_quantifier() {
        assert_parity(r#"[[ "aaa" =~ "^a{2,5}$" ]]; echo $?"#);
    }
}

mod pos_neg_combinations {
    use super::*;

    /// Combined start/end anchors.
    #[test]
    fn full_string_match_with_anchors() {
        assert_parity(r#"[[ "hello" =~ "^hello$" ]]; echo $?"#);
    }

    /// Anchored but with extra → no match.
    #[test]
    fn anchored_partial_no_match() {
        assert_parity(r#"[[ "helloworld" =~ "^hello$" ]]; echo $?"#);
    }
}

mod in_if_construct {
    use super::*;

    /// `if [[ str =~ pat ]]; then ...; fi`.
    #[test]
    fn regex_in_if_true() {
        assert_parity(
            r#"
if [[ "abc123" =~ "[0-9]+" ]]; then
  echo "yes: $MATCH"
else
  echo no
fi
"#,
        );
    }

    #[test]
    fn regex_in_if_false() {
        assert_parity(
            r#"
if [[ "abcdef" =~ "[0-9]+" ]]; then
  echo yes
else
  echo no
fi
"#,
        );
    }
}
