//! case / esac parity tests.

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

mod literal {
    use super::*;

    #[test]
    fn match_literal_first_arm() {
        assert_parity("case foo in foo) echo match;; *) echo nope;; esac");
    }

    #[test]
    fn match_literal_second_arm() {
        assert_parity("case bar in foo) echo nope;; bar) echo match;; esac");
    }

    #[test]
    fn no_match_no_output() {
        assert_parity("case xyz in foo) echo foo;; bar) echo bar;; esac; echo done");
    }

    #[test]
    fn match_default_star() {
        assert_parity("case xyz in foo) echo foo;; *) echo default;; esac");
    }

    #[test]
    fn empty_value_matches_empty_pattern() {
        assert_parity(r#"case "" in '') echo empty;; *) echo other;; esac"#);
    }

    #[test]
    fn whitespace_value() {
        assert_parity(r#"case " " in ' ') echo space;; *) echo other;; esac"#);
    }
}

mod glob_patterns {
    use super::*;

    #[test]
    fn star_prefix_match() {
        assert_parity(r#"case hello in hello*) echo prefix;; *) echo other;; esac"#);
    }

    #[test]
    fn star_suffix_match() {
        assert_parity(r#"case world in *world) echo suffix;; *) echo other;; esac"#);
    }

    #[test]
    fn question_one_char() {
        assert_parity(r#"case abc in a?c) echo match;; *) echo other;; esac"#);
    }

    #[test]
    fn bracket_char_class() {
        assert_parity(r#"case file7 in file[0-9]) echo digit;; *) echo other;; esac"#);
    }

    #[test]
    fn star_only_matches_anything() {
        assert_parity(r#"case anything in *) echo any;; esac"#);
    }
}

mod alternation {
    use super::*;

    #[test]
    fn pipe_alternation_first() {
        assert_parity(r#"case foo in foo|bar|baz) echo abc;; esac"#);
    }

    #[test]
    fn pipe_alternation_middle() {
        assert_parity(r#"case bar in foo|bar|baz) echo abc;; esac"#);
    }

    #[test]
    fn pipe_alternation_last() {
        assert_parity(r#"case baz in foo|bar|baz) echo abc;; esac"#);
    }

    #[test]
    fn pipe_alternation_no_match() {
        assert_parity(r#"case qux in foo|bar|baz) echo abc;; *) echo other;; esac"#);
    }

    #[test]
    fn alternation_with_glob() {
        assert_parity(r#"case file.rs in *.txt|*.md|*.rs) echo doc;; esac"#);
    }
}

mod terminators {
    use super::*;

    /// `;;` — match, run, then break out of case.
    #[test]
    fn double_semicolon_breaks_out() {
        assert_parity(
            r#"
case foo in
  foo) echo first;;
  *)   echo never;;
esac
"#,
        );
    }

    /// `;&` — match, run, then fall through to next arm (no test on
    /// next arm's pattern). zsh-extension.
    #[test]
    fn ampersand_semicolon_fall_through() {
        assert_parity(
            r#"
case foo in
  foo) echo first;&
  bar) echo second;&
  baz) echo third;;
esac
"#,
        );
    }

    /// `;;&` — match, run, then continue testing remaining patterns.
    /// zsh-extension.
    #[test]
    fn double_semicolon_amp_continue_match() {
        assert_parity(
            r#"
case foo in
  foo*) echo prefix;;&
  *foo) echo suffix;;&
  *)    echo other;;
esac
"#,
        );
    }
}

mod with_variables {
    use super::*;

    #[test]
    fn case_var_value() {
        assert_parity(r#"X=hello; case "$X" in hello) echo got;; esac"#);
    }

    #[test]
    fn case_arithmetic_expansion() {
        assert_parity(r#"case $((2+3)) in 5) echo five;; *) echo other;; esac"#);
    }

    #[test]
    fn case_cmdsubst_value() {
        assert_parity(r#"case $(echo abc) in abc) echo got;; esac"#);
    }
}

mod nested {
    use super::*;

    #[test]
    fn case_inside_case() {
        assert_parity(
            r#"
case top in
  top)
    case inner in
      inner) echo got-inner;;
    esac
    ;;
esac
"#,
        );
    }

    #[test]
    fn case_inside_if() {
        assert_parity(
            r#"
if true; then
  case foo in
    foo) echo got;;
  esac
fi
"#,
        );
    }
}

mod exit_status {
    use super::*;

    /// case exit status = last command's status from matched arm, or 0
    /// if no arm matched.
    #[test]
    fn exit_status_from_matched_arm() {
        assert_parity(r#"case foo in foo) false;; esac; echo $?"#);
    }

    #[test]
    fn exit_status_zero_when_no_match() {
        assert_parity(r#"case xyz in foo) false;; esac; echo $?"#);
    }
}

mod patterns_with_quotes {
    use super::*;

    /// Quoting in pattern disables glob meaning.
    #[test]
    fn quoted_star_is_literal() {
        assert_parity(r#"case '*' in '*') echo literal-star;; *) echo any;; esac"#);
    }

    #[test]
    fn quoted_question_is_literal() {
        assert_parity(r#"case '?' in '?') echo literal-q;; *) echo any;; esac"#);
    }
}

mod round_al_pins {
    use super::*;

    #[test]
    fn pipe_pattern_alternation() {
        assert_parity(r#"case word in (w|x) echo wx;; *) echo star;; esac"#);
    }

    #[test]
    fn multi_branch_numeric() {
        assert_parity(r#"case 1 in 1) echo one;; 2) echo two;; esac"#);
    }

    #[test]
    fn function_return_status() {
        assert_parity(r#"fn(){ return 3; }; fn; echo $?"#);
    }
}

/// Open-paren `(pat)` case arms — c:Src/parse.c:1321-1357 absorbs a
/// complete `(...)` as the whole pattern; the token after it is the
/// body's first word and must be lexed in command position
/// (c:1300-1302), so `out+=hit` there is an assignment, not a
/// command word.
mod open_paren_arms {
    use super::*;

    #[test]
    fn open_paren_literal_arm() {
        assert_parity(r#"case a in (a) print A;; (*) print other;; esac"#);
    }

    #[test]
    fn open_paren_alternation_arm() {
        assert_parity(r#"case c in (b|c) print BC;; (*) print other;; esac"#);
    }

    #[test]
    fn open_paren_spaced_alternation_arm() {
        assert_parity(r#"case e in ( d | e ) print DE;; (*) print other;; esac"#);
    }

    #[test]
    fn append_assignment_as_open_paren_arm_body() {
        assert_parity(r#"case a in (a) out+=hit;; esac; print $out"#);
    }

    #[test]
    fn array_append_as_open_paren_arm_body() {
        assert_parity(r#"case a in (a) arr+=(1 2);; esac; print $arr"#);
    }

    #[test]
    fn assoc_append_as_open_paren_arm_body() {
        assert_parity(r#"typeset -A h; case a in (a) h+=(k v);; esac; print ${(kv)h}"#);
    }
}
