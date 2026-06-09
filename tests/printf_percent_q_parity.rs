//! `printf %q` quoting round-trip parity vs zsh.
//!
//! `%q` should produce a shell-quoted form that — when fed back through the
//! shell — round-trips to the original byte string. zsh's policy escapes any
//! character the parser would otherwise treat specially: whitespace, glob
//! metas, sigils, operators, quotes, brackets, braces, etc.
//!
//! These pins lock in the cases zshrs matches. The previously-pinned
//! divergences (tab → `$'\t'`, mid-word `~` and `=`) are now fixed and
//! kept as regression tests in `mod divergences`.

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
    assert_eq!(z.exit, r.exit, "exit divergence on: {s}");
}

mod basics {
    use super::*;

    /// Plain alnum word: no escapes needed.
    #[test]
    fn plain_word_no_escape() {
        assert_parity(r#"printf "%q\n" plain"#);
    }

    /// Alphanumeric + digits stays bare.
    #[test]
    fn alphanumeric_no_escape() {
        assert_parity(r#"printf "%q\n" abc123"#);
    }

    /// Absolute path: `/` is safe, no escape.
    #[test]
    fn slash_path_no_escape() {
        assert_parity(r#"printf "%q\n" "/path/to/file""#);
    }

    /// Empty string → `''` (two single quotes), the shell-portable empty literal.
    #[test]
    fn empty_string_emits_quotes() {
        assert_parity(r#"printf "%q\n" """#);
    }
}

mod whitespace {
    use super::*;

    /// Space → backslash-space.
    #[test]
    fn space_backslash_escaped() {
        assert_parity(r#"printf "%q\n" "hello world""#);
    }

    /// Multiple spaces each get escaped.
    #[test]
    fn multiple_spaces_each_escaped() {
        assert_parity(r#"printf "%q\n" "a b c""#);
    }
}

mod metacharacters {
    use super::*;

    /// `*` glob meta → `\*`.
    #[test]
    fn glob_star_escaped() {
        assert_parity(r#"printf "%q\n" "a*b""#);
    }

    /// `?` glob meta → `\?`.
    #[test]
    fn glob_question_escaped() {
        assert_parity(r#"printf "%q\n" "a?b""#);
    }

    /// `[...]` glob char class → escape brackets.
    #[test]
    fn glob_brackets_escaped() {
        assert_parity(r#"printf "%q\n" "a[b]c""#);
    }

    /// `{...}` brace expansion → escape braces.
    #[test]
    fn braces_escaped() {
        assert_parity(r#"printf "%q\n" "a{b}c""#);
    }

    /// `$` sigil → `\$`.
    #[test]
    fn dollar_sigil_escaped() {
        assert_parity(r#"printf "%q\n" "a\$b""#);
    }

    /// `#` comment char → `\#`.
    #[test]
    fn hash_comment_escaped() {
        assert_parity(r#"printf "%q\n" "a#b""#);
    }
}

mod operators {
    use super::*;

    /// `;` command separator → `\;`.
    #[test]
    fn semicolon_pipe_amp_escaped() {
        assert_parity(r#"printf "%q\n" "a;b|c""#);
    }

    /// `&` background/and → `\&`.
    #[test]
    fn ampersand_escaped() {
        assert_parity(r#"printf "%q\n" "a&b""#);
    }

    /// `(` `)` subshell → escape both.
    #[test]
    fn parens_escaped() {
        assert_parity(r#"printf "%q\n" "a(b)c""#);
    }

    /// `>` redirect → `\>`.
    #[test]
    fn redirect_gt_escaped() {
        assert_parity(r#"printf "%q\n" "a>b""#);
    }

    /// `<` redirect → `\<`.
    #[test]
    fn redirect_lt_escaped() {
        assert_parity(r#"printf "%q\n" "a<b""#);
    }
}

mod quotes_and_backslash {
    use super::*;

    /// Embedded single-quote → escape with backslash.
    #[test]
    fn embedded_single_quote() {
        assert_parity(r#"printf "%q\n" "with'quote""#);
    }

    /// Embedded backslash literal.
    #[test]
    fn embedded_backslash() {
        assert_parity(r#"printf "%q\n" "back\\slash""#);
    }
}

mod multi_arg {
    use super::*;

    /// Two args reuse the format string; both go through `%q` independently.
    #[test]
    fn two_args_each_quoted_independently() {
        assert_parity(r#"printf "%q|%q\n" a b"#);
    }
}

mod round_trip {
    use super::*;

    /// The defining property of `%q`: output is round-trip safe.
    /// Quote `"hello world"`, then `eval` the quoted form into a variable,
    /// and the variable's value equals the original.
    #[test]
    fn space_round_trips_via_eval() {
        assert_parity(r#"X=$(printf "%q" "hello world"); eval "v=$X"; printf "[%s]\n" "$v""#);
    }

    /// Round-trip with multiple words.
    #[test]
    fn three_words_round_trip_via_eval() {
        assert_parity(r#"X=$(printf "%q" "with space tab"); eval "v=$X"; echo "[$v]""#);
    }
}

/// Previously pinned `#[ignore]` divergences, now fixed and pinned as
/// regression tests. Each comment documents the original gap and the
/// fix:
///   - tab → `$'\t'`: utils.rs:7077 routes ASCII control bytes through
///     the `$'…'` branch (the previous Rust port emitted `\<TAB>`).
///   - mid-word `~`/`=`: utils.rs:6301-6306 gate ported — only escape
///     at position 0, with MAGICEQUALSUBST + previous `=`/`:`, or for
///     `~` with EXTENDEDGLOB. The previous Rust port over-escaped both.
mod divergences {
    use super::*;

    /// zsh emits `tab$'\t'end` ($' ANSI-C form); regression pin
    /// (previously emitted `tab\<TAB>end`). Routes through the
    /// `c.is_ascii_control()` branch in `quotestring`.
    #[test]
    fn zshrs_bug_tab_uses_raw_backslash_not_dollar_quote() {
        assert_parity("printf \"%q\\n\" \"tab\tend\"");
    }

    /// `~` mid-word is bare per zsh's c:6306 gate (only escape if
    /// position 0 or EXTENDEDGLOB). Regression pin for the
    /// quotestring `=`/`~` gate.
    #[test]
    fn zshrs_bug_tilde_over_escaped() {
        assert_parity(r#"printf "%q\n" "a~b""#);
    }

    /// `=` not at first character is bare per zsh's c:6302-6305
    /// gate (only escape if position 0 or MAGICEQUALSUBST with
    /// preceding `=`/`:`). Regression pin for the quotestring
    /// `=`/`~` gate.
    #[test]
    fn zshrs_bug_equals_after_first_char_over_escaped() {
        assert_parity(r#"printf "%q\n" "key=value""#);
    }
}
