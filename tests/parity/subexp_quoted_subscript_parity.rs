//! A subscript or `#`/`%` operator applied to a QUOTED nested expansion.
//!
//! `${"${(f)v}"[2]}` — an inner substitution wrapped in double quotes,
//! carrying a split flag, then indexed — must index the SPLIT ARRAY.
//!
//! C reaches that by stepping the body pointer past the closing quote
//! marker once the inner subexp has been expanded:
//!
//! ```text
//! c:Src/subst.c:2693-2698
//!     /*
//!      * This tests for the second double quote in an expression
//!      * like ${(f)"$(<file)"}, compare above.
//!      */
//!     while (inull(*s))
//!         s++;
//! ```
//!
//! Only after that skip does C run the subscript loop
//! (c:2868 `while (v || ((inbrace || …) && isbrack(*s)))`, applying the
//! index at c:2900 via `getindex`) and the operator gate. zshrs dropped
//! the quote markers only in the DERIVED `rest` string and left the body
//! index parked on the `Dnull`, so `body_chars[idx] == '['` (c:2867) and
//! the `#`/`%` operator tests silently declined and the whole joined
//! value was returned — wrong DATA, with no diagnostic.
//!
//! Measured before the fix (`i=$'a\nb\nc'`):
//!
//! ```text
//!   ${"${(f)i}"[2]}   zsh: b        zshrs: a b c
//!   ${"${(f)i}"[-1]}  zsh: c        zshrs: a b c
//!   ${"${(f)i}"[1,2]} zsh: a b      zshrs: a b c
//!   ${"${(f)i}"#aa}   zsh: ` bb cc` zshrs: aa bb cc   (unstripped)
//! ```
//!
//! `/` was unaffected because the replace path scans the marker-stripped
//! `rest` instead of testing the raw body character, and the UNQUOTED
//! form `${${(f)i}[2]}` was always correct — which is why this survived
//! until a completer used the quoted spelling.
//!
//! The live symptom was `_git`'s `__git_worktrees`
//! (Completion/Unix/Command/_git:8866-8888), which parses
//! `git worktree list --porcelain` records with
//!
//! ```text
//!   hash=${${${"${(f)i}"[2]}#HEAD }[1,9]}
//!   branch=${${"${(f)i}"[3]}#branch refs/heads/}
//! ```
//!
//! With the subscript dropped, every record's hash and branch became the
//! whole record text, corrupting the `_describe` descriptions for
//! `git worktree move <TAB>` and `git worktree unlock <TAB>`.

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

/// (stdout, exit code)
fn run_zsh(script: &str) -> (String, Option<i32>) {
    let o = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("spawn zsh");
    (String::from_utf8_lossy(&o.stdout).into_owned(), o.status.code())
}

fn run_zshrs(script: &str) -> (String, Option<i32>) {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .output()
        .expect("spawn zshrs");
    (String::from_utf8_lossy(&o.stdout).into_owned(), o.status.code())
}

/// Differential: the reference shell defines the expected result, so this
/// pins BEHAVIOUR rather than a string someone typed from memory.
fn assert_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let (zout, zexit) = run_zsh(script);
    let (rout, rexit) = run_zshrs(script);
    assert_eq!(
        zout, rout,
        "stdout divergence on:\n{script}\n--- zsh ---\n{zout:?}\n--- zshrs ---\n{rout:?}"
    );
    assert_eq!(
        zexit, rexit,
        "exit-status divergence on:\n{script}\n zsh={zexit:?} zshrs={rexit:?}"
    );
}

/// `i=$'a\nb\nc'` then the expression under test.
fn with_lines(expr: &str) -> String {
    format!("i=$'a\\nb\\nc'; print -r -- {expr}")
}

#[test]
fn quoted_split_indexed_returns_the_element_not_the_join() {
    assert_parity(&with_lines(r#"${"${(f)i}"[2]}"#));
}

#[test]
fn quoted_split_first_and_last_index() {
    assert_parity(&with_lines(r#"${"${(f)i}"[1]}"#));
    assert_parity(&with_lines(r#"${"${(f)i}"[-1]}"#));
}

#[test]
fn quoted_split_range_subscript() {
    assert_parity(&with_lines(r#"${"${(f)i}"[1,2]}"#));
}

#[test]
fn quoted_split_with_at_flag_is_indexed_too() {
    assert_parity(&with_lines(r#"${"${(@f)i}"[2]}"#));
}

/// The control: the UNQUOTED spelling was never broken, so a regression
/// here means the fix moved the wrong index.
#[test]
fn unquoted_split_indexed_is_unchanged() {
    assert_parity(&with_lines(r#"${${(f)i}[2]}"#));
}

/// `(ps.…)` is the same split machinery spelled explicitly.
#[test]
fn quoted_ps_flag_split_is_indexed() {
    assert_parity(&with_lines(r#"${"${(ps.\n.)i}"[2]}"#));
}

#[test]
fn strip_operators_apply_after_a_quoted_subexp() {
    let s = "i=$'aa\\nbb\\ncc'; print -r -- ";
    assert_parity(&format!(r##"{s}${{"${{(f)i}}"#aa}}"##));
    assert_parity(&format!(r##"{s}${{"${{(f)i}}"%cc}}"##));
}

/// `/` already worked (it scans the marker-stripped rest); pinned so the
/// fix does not regress the one arm that was correct.
#[test]
fn replace_operator_after_a_quoted_subexp_still_works() {
    let s = "i=$'aa\\nbb\\ncc'; print -r -- ";
    assert_parity(&format!(r#"{s}${{"${{(f)i}}"/aa/XX}}"#));
}

/// The exact `__git_worktrees` shape: index a quoted split, then strip a
/// prefix from the element, then take a character range.
#[test]
fn git_worktrees_record_parse_shape() {
    let rec = "i=$'worktree /p/q\\nHEAD abcdef0123456789\\nbranch refs/heads/main'; print -r -- ";
    assert_parity(&format!(r#"{rec}${{${{${{"${{(f)i}}"[2]}}#HEAD }}[1,9]}}"#));
    assert_parity(&format!(
        r#"{rec}${{${{"${{(f)i}}"[3]}}#branch refs/heads/}}"#
    ));
}
