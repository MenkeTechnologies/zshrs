//! `${(Q)v}` — a backslash with nothing after it is a quote character with
//! nothing to quote, and it disappears.
//!
//! `(Q)` is `quotemod--` (c:Src/subst.c:2263) and the negative arm at
//! c:Src/subst.c:4137-4152 is:
//!
//! ```c
//! haserr = parse_subst_string(val);
//! ...
//! remnulargs(val);
//! untokenize(val);
//! ```
//!
//! `parse_subst_string` (c:Src/lex.c:1796) re-lexes the value in `sub = 1`
//! mode, and a backslash lands in `case LX2_BKSLASH:` (c:Src/lex.c:1261):
//!
//! ```c
//! case LX2_BKSLASH:
//!     c = hgetc();
//!     if (c == '\n') { ... } else {
//!         add(Bnull);
//!         ...
//!     }
//!     if (lexstop)
//!         goto brk;
//! ```
//!
//! The char AFTER the backslash is read FIRST. When the backslash is the last
//! character that read hits end-of-input and sets `lexstop`, so the only thing
//! appended is the `Bnull` marker and the loop breaks before any character is
//! added. `remnulargs` (c:Src/glob.c:3659) then deletes every `inull(c)` —
//! `Snull`/`Dnull`/`Bnull` — and the backslash is gone:
//!
//! ```
//! % zsh -f -c 'v="\\"; print -r -- ${(Q)v}' | xxd
//! 00000000: 0a
//! ```
//!
//! The port's `unquote_one` consumed `\<char>` pairs but fell through to a
//! literal push when the backslash had no partner, so the backslash survived
//! (`5c 0a`). Every row below is byte-compared against the real `zsh`.
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

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

fn run(bin: &str, args: &[&str], s: &str) -> (Vec<u8>, i32) {
    let o = Command::new(bin)
        .args(args)
        .arg(s)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn shell");
    (o.stdout, o.status.code().unwrap_or(-1))
}

/// Byte parity for one script against the real zsh. Compares raw bytes, not
/// a lossy `String` — the whole defect is one stray `0x5c`.
fn byte_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let rs = zshrs_bin();
    let rs = rs.to_str().expect("utf-8 path");
    let (zo, zx) = run(zsh_path(), &["-f", "-c"], s);
    let (ro, rx) = run(rs, &["--zsh", "-f", "-c"], s);
    assert_eq!(
        zo,
        ro,
        "stdout bytes diverge:\n{s}\n--- zsh   --- {:02x?}\n--- zshrs --- {:02x?}",
        zo,
        ro
    );
    assert_eq!(zx, rx, "exit status diverges:\n{s}");
}

/// `qv` is assigned via `$'…'` so the test source says exactly which bytes the
/// parameter holds, with no double layer of shell quoting to reason about.
fn set_and_print(value_literal: &str, expansion: &str) -> String {
    format!("qv={value_literal}\nprint -r -- {expansion}")
}

/// The reported defect and its immediate neighbours.
mod trailing_backslash_is_dropped {
    use super::*;

    /// `v="\"` — the backslash is the whole value. zsh prints one newline.
    #[test]
    fn lone_backslash_expands_to_nothing() {
        byte_parity(&set_and_print(r"$'\\'", "${(Q)qv}"));
    }

    /// Its length is 0, not 1.
    #[test]
    fn lone_backslash_length_is_zero() {
        byte_parity(&set_and_print(r"$'\\'", "${#${(Q)qv}}"));
    }

    /// `a\` keeps the `a` and drops the backslash.
    #[test]
    fn text_then_trailing_backslash() {
        byte_parity(&set_and_print(r"$'a\\'", "${(Q)qv}"));
    }

    #[test]
    fn text_then_trailing_backslash_length() {
        byte_parity(&set_and_print(r"$'a\\'", "${#${(Q)qv}}"));
    }

    /// Three backslashes: the first quotes the second, the third is the lone
    /// trailing one. One backslash survives, not two.
    #[test]
    fn three_backslashes_leave_one() {
        byte_parity(&set_and_print(r"$'\\\\\\'", "${(Q)qv}"));
    }

    /// A quoted pair followed by the lone one: `\ \` is a space, then nothing.
    #[test]
    fn quoted_space_then_trailing_backslash() {
        byte_parity(&set_and_print(r"$'\\ \\'", "${(Q)qv}"));
    }

    /// The value is multibyte, so a byte-level truncation would show up here.
    #[test]
    fn multibyte_then_trailing_backslash() {
        byte_parity(&set_and_print("$'\u{e9}\\\\'", "${(Q)qv}"));
    }

    /// Inside double quotes the result is one word, still with no backslash.
    #[test]
    fn trailing_backslash_inside_double_quotes() {
        byte_parity(&set_and_print(r"$'a\\'", "\"${(Q)qv}\""));
    }
}

/// The pairs that must NOT change — the fix only touches the partnerless case.
mod paired_backslashes_unchanged {
    use super::*;

    /// `\\` is one quoted backslash and stays one backslash.
    #[test]
    fn escaped_backslash_survives() {
        byte_parity(&set_and_print(r"$'\\\\'", "${(Q)qv}"));
    }

    /// `\a` drops the quote character and keeps the `a`.
    #[test]
    fn backslash_before_letter() {
        byte_parity(&set_and_print(r"$'\\a'", "${(Q)qv}"));
    }

    /// A backslash in the MIDDLE has a partner and is consumed normally.
    #[test]
    fn backslash_between_letters() {
        byte_parity(&set_and_print(r"$'a\\b'", "${(Q)qv}"));
    }

    /// `'…'` and `"…"` spans are handled by their own arms and are untouched.
    #[test]
    fn single_quoted_span() {
        byte_parity(&set_and_print(r#"$'\'abc\''"#, "${(Q)qv}"));
    }

    #[test]
    fn double_quoted_span() {
        byte_parity(&set_and_print(r#"$'"abc"'"#, "${(Q)qv}"));
    }

    /// An orphan quote stays literal (Bug #507's contract).
    #[test]
    fn orphan_double_quote_stays_literal() {
        byte_parity(&set_and_print(r#"$'x"y'"#, "${(Q)qv}"));
    }
}

/// `(Q)` iterates `aval` per element (c:Src/subst.c:4095-4108), and an element
/// that dequotes to nothing becomes `Nularg` (c:Src/glob.c:3686) — which an
/// unquoted splat elides. So the three-element array below is two words.
mod arrays {
    use super::*;

    const ARR: &str = "qa=( $'\\\\' $'a\\\\' $'\\\\\\\\' )";

    #[test]
    fn per_element_dequote() {
        byte_parity(&format!("{ARR}\nprint -r -- ${{(Q)qa}}"));
    }

    #[test]
    fn emptied_element_is_elided_from_the_count() {
        byte_parity(&format!("{ARR}\nprint -r -- ${{#${{(Q)qa}}}}"));
    }

    #[test]
    fn quoted_splat_keeps_every_element() {
        byte_parity(&format!("{ARR}\nprint -rl -- \"${{(@Q)qa}}\""));
    }
}

/// `${(Q)${(q)v}}` must return the original value for every quoting style.
mod round_trips {
    use super::*;

    /// Values chosen so each one exercises a different arm of the dequoter:
    /// whitespace, an embedded backslash, a TRAILING backslash, a lone
    /// backslash, embedded quotes of both kinds, and a tab.
    const VALUES: &[&str] = &[
        r"$'a b'",
        r"$'a\\b'",
        r"$'a\\'",
        r"$'\\'",
        r#"$'a\'b'"#,
        r#"$'a"b'"#,
        r"$'a\tb'",
        r"$'\\ \\'",
        r"$'a\\\\b'",
    ];

    fn round_trip(quote_flag: &str) {
        for v in VALUES {
            byte_parity(&set_and_print(v, &format!("${{(Q)${{({quote_flag})qv}}}}")));
        }
    }

    #[test]
    fn through_q() {
        round_trip("q");
    }

    #[test]
    fn through_qq() {
        round_trip("qq");
    }

    #[test]
    fn through_qqq() {
        round_trip("qqq");
    }

    #[test]
    fn through_qqqq() {
        round_trip("qqqq");
    }

    #[test]
    fn through_q_minus() {
        round_trip("q-");
    }

    #[test]
    fn through_q_plus() {
        round_trip("q+");
    }

    /// The flags in one expansion, both orders — `(Q)` and `(q)` cancel to
    /// `quotemod == 0`, so neither runs.
    #[test]
    fn q_and_big_q_in_one_flag_list() {
        for v in VALUES {
            byte_parity(&set_and_print(v, "${(Qq)qv}"));
            byte_parity(&set_and_print(v, "${(qQ)qv}"));
        }
    }
}
